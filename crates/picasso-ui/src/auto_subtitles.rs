//! Timeline > Create Subtitles from Audio: Whisper (`picasso_asr`) listens to the sound of the clip
//! being worked on, on this computer, and what is said is put on a new subtitle track. The model
//! is downloaded the first time it is wanted. The work runs on a thread of its own and the
//! window shows how far it is, with a button to stop.

// The browser build has no sound files to read and no speech model.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::{Context, RichText};
use picasso_core::ClipId;

use crate::app::StudioApp;
use crate::subtitles::Cue as FrameCue;

/// How much of a clip's sound is read in one go, in seconds: half an hour.
pub(crate) const MAX_SECONDS: f64 = 1800.0;
/// The longest a subtitle stays, in seconds.
const MAX_CUE_SECONDS: f64 = 5.0;

/// One thing heard: when it starts and ends (milliseconds into the sound read) and the words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Heard {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// What a running job reports.
#[derive(Clone, Debug, Default)]
pub(crate) struct JobState {
    pub what: String,
    pub share: f32,
    /// Set when the job has ended: the cues, or what went wrong in words.
    pub result: Option<Result<Vec<Heard>, String>>,
    pub downloading: bool,
}

/// A job on its own thread.
pub(crate) struct Job {
    state: Arc<Mutex<JobState>>,
    stop: Arc<AtomicBool>,
}

impl Job {
    fn snapshot(&self) -> JobState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// The window's state.
pub(crate) struct AutoSubtitles {
    pub clip: ClipId,
    /// Which of `picasso_asr::models::MODELS`.
    pub model: usize,
    /// `None`: let the model hear the language.
    pub language: Option<String>,
    pub max_words: usize,
    pub job: Option<Job>,
    pub problem: Option<String>,
}

/// Where the models are kept: `PICASSO_MODELS_DIR` when it is set, else in the user's data folder.
pub(crate) fn models_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PICASSO_MODELS_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::project_manager::data_dir().map(|d| d.join("models"))
}

/// The name people know a language by, for the codes of the common ones; the code for the rest.
pub(crate) fn language_name(code: &str) -> String {
    let name = match code {
        "en" => "English",
        "zh" => "Chinese",
        "de" => "German",
        "es" => "Spanish",
        "ru" => "Russian",
        "ko" => "Korean",
        "fr" => "French",
        "ja" => "Japanese",
        "pt" => "Portuguese",
        "tr" => "Turkish",
        "pl" => "Polish",
        "ca" => "Catalan",
        "nl" => "Dutch",
        "ar" => "Arabic",
        "sv" => "Swedish",
        "it" => "Italian",
        "id" => "Indonesian",
        "hi" => "Hindi",
        "fi" => "Finnish",
        "vi" => "Vietnamese",
        "he" => "Hebrew",
        "uk" => "Ukrainian",
        "el" => "Greek",
        "ms" => "Malay",
        "cs" => "Czech",
        "ro" => "Romanian",
        "da" => "Danish",
        "hu" => "Hungarian",
        "ta" => "Tamil",
        "no" => "Norwegian",
        "th" => "Thai",
        "ur" => "Urdu",
        "hr" => "Croatian",
        "bg" => "Bulgarian",
        "lt" => "Lithuanian",
        "sk" => "Slovak",
        "bn" => "Bengali",
        "si" => "Sinhala",
        "tl" => "Tagalog",
        "sr" => "Serbian",
        "sl" => "Slovenian",
        _ => return code.to_string(),
    };
    format!("{name} ({code})")
}

/// The cues of what was heard as subtitles of the timeline: a clip that starts at timeline frame
/// `start`, `duration` frames long, shows its file at `speed` times the speed (the times of the
/// file are scaled to the timeline), `seconds_per_frame` is the project's. Cues past the end of the
/// clip are dropped and the last is cut to it.
pub(crate) fn frame_cues(
    heard: &[Heard],
    (start, duration): (u64, u64),
    seconds_per_frame: f64,
    speed: f64,
) -> Vec<FrameCue> {
    let frame_of = |ms: u64| start + (ms as f64 / 1000.0 / speed.max(0.01) / seconds_per_frame).round() as u64;
    let end = start + duration;
    let mut out: Vec<FrameCue> = Vec::new();
    for cue in heard {
        let (a, b) = (frame_of(cue.start_ms), frame_of(cue.end_ms));
        if a >= end {
            continue;
        }
        // At least one frame, and never over the next cue.
        let b = b.max(a + 1).min(end);
        if let Some(last) = out.last_mut() {
            if last.end > a {
                last.end = a.max(last.start + 1);
            }
        }
        out.push(FrameCue {
            start: a,
            end: b,
            text: cue.text.clone(),
        });
    }
    out.retain(|c| c.end > c.start && !c.text.trim().is_empty());
    out
}

/// The sound of `path` from `from` seconds for at most `seconds`, mono at 16 kHz (what Whisper
/// takes), read in pieces so that what is held at once stays small. `progress` is told how far
/// (0 to 1) and gives `false` to stop.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sound_16k(
    path: &str,
    from: f64,
    seconds: f64,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Vec<f32>, String> {
    use picasso_decode::audio::{AudioDecoder, SAMPLE_RATE};
    let mut decoder = AudioDecoder::open(Path::new(path))
        .map_err(|e| format!("{path}: {e}"))?
        .ok_or_else(|| format!("{path} has no sound"))?;
    let every = (SAMPLE_RATE / 16_000) as usize;
    let first = (from.max(0.0) * f64::from(SAMPLE_RATE)).round() as u64;
    let total = (seconds.clamp(0.0, MAX_SECONDS) * f64::from(SAMPLE_RATE)).round() as usize;
    let mut out = Vec::with_capacity(total / every + 1);
    let piece = SAMPLE_RATE as usize * 10;
    let mut done = 0;
    while done < total {
        if !progress(done as f32 / total.max(1) as f32) {
            return Err("cancelled".into());
        }
        let count = piece.min(total - done);
        let stereo = decoder
            .read_at(first + done as u64, count)
            .map_err(|e| format!("{path}: {e}"))?;
        let mono: Vec<f32> = stereo.chunks_exact(2).map(|s| (s[0] + s[1]) * 0.5).collect();
        out.extend(mono.chunks(every).map(|c| c.iter().sum::<f32>() / c.len() as f32));
        done += count;
    }
    Ok(out)
}

/// Reads the sound of a file and what is said in it: everything the job does, with no window in
/// it. `progress(share, what)` gives `false` to stop.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn transcribe_file(
    models: &Path,
    model: &picasso_asr::Model,
    path: &str,
    (from, seconds): (f64, f64),
    language: Option<&str>,
    max_words: usize,
    progress: &mut dyn FnMut(f32, &str) -> bool,
) -> Result<Vec<Heard>, String> {
    let sound = sound_16k(path, from, seconds, &mut |share| {
        progress(share * 0.1, "Reading the sound")
    })?;
    if !progress(0.1, "Loading the model") {
        return Err("cancelled".into());
    }
    let mut whisper = picasso_asr::Whisper::load(&picasso_asr::models::dir_of(models, model))
        .map_err(|e| e.to_string())?;
    let transcript = whisper
        .transcribe(&sound, language, &mut |share, what| progress(0.1 + share * 0.9, what))
        .map_err(|e| e.to_string())?;
    Ok(picasso_asr::cues::pack(&transcript.segments, max_words, MAX_CUE_SECONDS)
        .into_iter()
        .map(|c| Heard {
            start_ms: c.start_ms,
            end_ms: c.end_ms,
            text: c.text,
        })
        .collect())
}

impl StudioApp {
    /// Whether there is a clip with a file whose sound can be read.
    pub(crate) fn can_auto_subtitle(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.target_clip()
                .is_some_and(|id| crate::waveform_align::sound_path(self.project(), id).is_some())
        }
    }

    /// Timeline > Create Subtitles from Audio.
    pub(crate) fn open_auto_subtitles(&mut self) {
        let Some(clip) = self.target_clip() else {
            self.fail("select a clip with sound first");
            return;
        };
        if !self.can_auto_subtitle() {
            self.fail("this clip has no file to take the sound from");
            return;
        }
        // The first model that is already there, else the smallest that knows many languages.
        #[cfg(not(target_arch = "wasm32"))]
        let model = {
            let models = models_dir();
            models
                .as_ref()
                .and_then(|dir| picasso_asr::models::MODELS.iter().position(|m| picasso_asr::models::is_installed(dir, m)))
                .or_else(|| picasso_asr::models::MODELS.iter().position(|m| m.multilingual))
                .unwrap_or(0)
        };
        #[cfg(target_arch = "wasm32")]
        let model = 0;
        self.extra.auto_subtitles = Some(AutoSubtitles {
            clip,
            model,
            language: None,
            max_words: 7,
            job: None,
            problem: None,
        });
    }

    /// Puts the heard cues on a new subtitle track: one undo step.
    pub(crate) fn place_heard_subtitles(&mut self, clip: ClipId, heard: &[Heard]) -> usize {
        let project = self.project();
        let Some((_, c)) = project.clip(clip) else {
            return 0;
        };
        let seconds_per_frame = f64::from(project.fps.den) / f64::from(project.fps.num.max(1));
        let cues = frame_cues(heard, (c.start, c.duration), seconds_per_frame, f64::from(c.speed));
        let count = cues.len();
        if count == 0 {
            return 0;
        }
        self.place_subtitles("the sound", cues);
        count
    }

    /// Draws the window while it is open, and takes the result of a job that has ended.
    pub(crate) fn auto_subtitles_window(&mut self, ctx: &Context) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(mut st) = self.extra.auto_subtitles.take() else {
                return;
            };
            // A job that has ended: its result is taken (a download goes on to the next step by hand).
            let mut finished = None;
            if let Some(job) = &st.job {
                let snap = job.snapshot();
                if let Some(result) = snap.result {
                    finished = Some((result, snap.downloading));
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(150));
                }
            }
            if let Some((result, was_download)) = finished {
                st.job = None;
                match result {
                    Err(e) if e == "cancelled" => st.problem = None,
                    Err(e) => st.problem = Some(e),
                    Ok(_) if was_download => st.problem = None,
                    Ok(heard) => {
                        let count = self.place_heard_subtitles(st.clip, &heard);
                        if count == 0 {
                            st.problem = Some("no speech was heard in this clip".into());
                        } else {
                            self.info(format!("Created {count} subtitles from the sound"));
                            return;
                        }
                    }
                }
            }
            let models = models_dir();
            let model = picasso_asr::models::MODELS[st.model.min(picasso_asr::models::MODELS.len() - 1)];
            let installed = models
                .as_ref()
                .is_some_and(|dir| picasso_asr::models::is_installed(dir, &model));
            let mut open = true;
            let mut start_download = false;
            let mut start_transcribe = false;
            let mut stop = false;
            let clip_name = self
                .project()
                .clip(st.clip)
                .map(|(_, c)| c.name.clone())
                .unwrap_or_default();
            egui::Window::new("Create Subtitles from Audio")
                .id(egui::Id::new("auto_subtitles_window"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(430.0)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Listens to the sound of {clip_name} and puts what is said on a new subtitle track. \
                         It runs on this computer: the sound is not sent anywhere."
                    ));
                    ui.add_space(6.0);
                    let busy = st.job.is_some();
                    ui.add_enabled_ui(!busy, |ui| {
                        egui::ComboBox::from_label("Model")
                            .selected_text(model.label)
                            .show_ui(ui, |ui| {
                                for (i, m) in picasso_asr::models::MODELS.iter().enumerate() {
                                    let here = models
                                        .as_ref()
                                        .is_some_and(|d| picasso_asr::models::is_installed(d, m));
                                    let tag = if here { "installed".to_string() } else { format!("{} MB", m.bytes() / 1_000_000) };
                                    ui.selectable_value(&mut st.model, i, format!("{} - {} ({tag})", m.label, m.note));
                                }
                            });
                        if model.multilingual {
                            let shown = st
                                .language
                                .as_deref()
                                .map_or("Detect the language".to_string(), language_name);
                            egui::ComboBox::from_label("Language")
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut st.language, None, "Detect the language");
                                    for code in picasso_asr::languages() {
                                        ui.selectable_value(&mut st.language, Some(code.clone()), language_name(&code));
                                    }
                                });
                        } else {
                            ui.label("Language: English");
                        }
                        ui.add(egui::Slider::new(&mut st.max_words, 3..=14).text("Words in a subtitle"));
                    });
                    ui.add_space(6.0);
                    if let Some(job) = &st.job {
                        let snap = job.snapshot();
                        ui.label(&snap.what);
                        ui.add(egui::ProgressBar::new(snap.share.clamp(0.0, 1.0)).show_percentage());
                        if ui.button("Stop").clicked() {
                            stop = true;
                        }
                    } else if installed {
                        if ui.button(RichText::new("Create Subtitles").strong()).clicked() {
                            start_transcribe = true;
                        }
                    } else {
                        ui.label(format!(
                            "The {} model is downloaded once ({} MB) and kept for next time.",
                            model.label,
                            model.bytes() / 1_000_000
                        ));
                        if ui.button(RichText::new("Download the model").strong()).clicked() {
                            start_download = true;
                        }
                    }
                    if let Some(problem) = &st.problem {
                        ui.add_space(4.0);
                        ui.colored_label(egui::Color32::from_rgb(230, 120, 100), problem);
                    }
                });
            if stop {
                if let Some(job) = &st.job {
                    job.stop.store(true, Ordering::Relaxed);
                }
            }
            if start_download || start_transcribe {
                st.problem = None;
                match self.start_job(&st, model, start_download) {
                    Ok(job) => st.job = Some(job),
                    Err(e) => st.problem = Some(e),
                }
            }
            if open {
                self.extra.auto_subtitles = Some(st);
            } else if let Some(job) = &st.job {
                // Closing the window stops what it was doing.
                job.stop.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Starts the download of `model`, or the reading of the clip's sound with it.
    #[cfg(not(target_arch = "wasm32"))]
    fn start_job(&mut self, st: &AutoSubtitles, model: picasso_asr::Model, download: bool) -> Result<Job, String> {
        let models = models_dir().ok_or("there is no folder to keep the model in")?;
        let state = Arc::new(Mutex::new(JobState {
            what: if download { "Downloading the model".into() } else { "Starting".into() },
            downloading: download,
            ..JobState::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, flag) = (state.clone(), stop.clone());
        let report = move |share: f32, what: &str| -> bool {
            if let Ok(mut s) = shared.lock() {
                s.share = share;
                if !what.is_empty() {
                    s.what = what.to_string();
                }
            }
            !flag.load(Ordering::Relaxed)
        };
        let finish = {
            let shared = state.clone();
            move |result: Result<Vec<Heard>, String>| {
                if let Ok(mut s) = shared.lock() {
                    s.result = Some(result);
                }
            }
        };
        if download {
            std::thread::spawn(move || {
                let mut report = report;
                let result = picasso_asr::models::download(&models, &model, &mut |done, total| {
                    let mb = |b: u64| b / 1_000_000;
                    report(done as f32 / total.max(1) as f32, &format!("Downloading the model: {} of {} MB", mb(done), mb(total)))
                })
                .map(|()| Vec::new())
                .map_err(|e| e.to_string());
                finish(result);
            });
        } else {
            let project = self.project();
            let (_, clip) = project.clip(st.clip).ok_or("the clip is gone")?;
            let path = crate::waveform_align::sound_path(project, st.clip)
                .ok_or("this clip has no file to take the sound from")?;
            if clip.reverse || clip.freeze {
                return Err("a reversed or frozen clip has no speech to hear".into());
            }
            let spf = f64::from(project.fps.den) / f64::from(project.fps.num.max(1));
            let speed = f64::from(clip.speed).max(0.01);
            let from = clip.source_in as f64 * spf;
            let shown = clip.duration as f64 * spf * speed;
            let (language, max_words) = (st.language.clone(), st.max_words);
            std::thread::spawn(move || {
                let mut report = report;
                let result = transcribe_file(
                    &models,
                    &model,
                    &path,
                    (from, shown.min(MAX_SECONDS)),
                    language.as_deref(),
                    max_words,
                    &mut report,
                );
                finish(result);
            });
        }
        Ok(Job { state, stop })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(spans: &[(u64, u64, &str)]) -> Vec<Heard> {
        spans
            .iter()
            .map(|&(a, b, t)| Heard {
                start_ms: a,
                end_ms: b,
                text: t.into(),
            })
            .collect()
    }

    #[test]
    fn what_was_heard_lands_on_the_timeline_frames_of_the_clip() {
        // A clip at frame 100, 250 frames long (10 seconds), 25 fps (0.04 s a frame), normal speed.
        let cues = frame_cues(
            &heard(&[(1000, 3000, "Hello"), (3000, 5000, "there"), (9000, 12_000, "late"), (11_000, 12_000, "past")]),
            (100, 250),
            0.04,
            1.0,
        );
        // The one that starts after the clip ends is dropped; the one that runs over is cut to it.
        assert_eq!(cues.len(), 3);
        assert_eq!((cues[0].start, cues[0].end), (125, 175));
        assert_eq!((cues[1].start, cues[1].end), (175, 225));
        assert_eq!((cues[2].start, cues[2].end), (325, 350));
        let beyond = frame_cues(&heard(&[(20_000, 21_000, "x")]), (100, 250), 0.04, 1.0);
        assert!(beyond.is_empty());
    }

    #[test]
    fn the_times_follow_the_speed_of_the_clip_and_never_overlap() {
        // At twice the speed the sound passes in half the time.
        let fast = frame_cues(&heard(&[(4000, 6000, "Quick")]), (0, 1000), 0.04, 2.0);
        assert_eq!((fast[0].start, fast[0].end), (50, 75));
        // Overlapping cues are cut where the next begins; a blank cue goes.
        let tangled = frame_cues(&heard(&[(0, 3000, "One"), (2000, 4000, "Two"), (4000, 5000, "  ")]), (0, 1000), 0.04, 1.0);
        assert_eq!(tangled.len(), 2);
        assert_eq!(tangled[0].end, tangled[1].start);
        assert!(tangled.iter().all(|c| c.end > c.start));
    }

    #[test]
    fn language_names_are_for_people_and_fall_back_to_the_code() {
        assert_eq!(language_name("de"), "German (de)");
        assert_eq!(language_name("haw"), "haw");
    }
}
