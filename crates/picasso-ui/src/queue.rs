//! The render queue: exports wait in a list and run one after another on a thread of their own.
//! Each one keeps the project as it was when it was added, so editing goes on while it renders.
//! The browser build has no encoders, so there the page only says so.

// Folder browsing, render settings and the queue only run on the desktop; the browser build keeps
// the types so the page still draws.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

/// How a job of the render queue stands, as the page shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JobState {
    Queued,
    Running,
    Done(f32),
    Failed(String),
    Cancelled,
}

/// One job of the render queue, as the page shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Job {
    pub id: u64,
    pub name: String,
    pub label: String,
    pub state: JobState,
    pub fraction: f32,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::Deliver;

#[cfg(target_arch = "wasm32")]
mod web {
    use egui::{Context, RichText, Ui};

    use crate::app::{Status, StudioApp};
    use crate::theme;

    #[derive(Default)]
    pub(crate) struct Deliver;

    impl Deliver {
        pub(crate) fn poll(&mut self, _status: &mut Status, _ctx: &Context) {}
    }

    impl StudioApp {
        pub(crate) fn enqueue_export(&mut self, _path: String) {}

        pub(crate) fn jobs(&self) -> Vec<super::Job> {
            Vec::new()
        }
        pub(crate) fn cancel_job(&mut self, _id: u64) {}
        pub(crate) fn remove_job(&mut self, _id: u64) {}
        pub(crate) fn clear_finished_jobs(&mut self) {}
        pub(crate) fn render_all(&mut self, _ctx: &Context) {}
        pub(crate) fn render_busy(&self) -> bool {
            false
        }
        pub(crate) fn add_to_queue(&mut self, _ctx: &Context) {}
        pub(crate) fn estimated_mb(&self) -> f32 {
            0.0
        }
        pub(crate) fn can_render_here(&self) -> bool {
            false
        }
        pub(crate) fn range_is_whole(&self) -> bool {
            true
        }
        pub(crate) fn set_range_whole(&mut self, _whole: bool) {}

        #[allow(dead_code)]
        pub(crate) fn render_note(&self, ui: &mut Ui) {
            ui.label(
                RichText::new(
                    "Exporting video and audio needs the desktop app; the browser can export single frames.",
                )
                .color(theme::TEXT_DIM),
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use web::Deliver;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod native {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use egui::Context;
    use picasso_core::Project;
    use picasso_export::{render, ExportError, Options, Preset, Settings};
    use picasso_render::MediaCache;

    use crate::app::{Status, StudioApp};
    use crate::platform;
    use tracing::warn;

    /// How far an item of the queue is.
    #[derive(Clone, Debug, PartialEq)]
    pub(crate) enum State {
        Queued,
        Running,
        Done(Duration),
        Failed(String),
        Cancelled,
    }

    struct Item {
        id: u64,
        name: String,
        settings: Settings,
        project: Arc<Project>,
        cache: MediaCache,
        state: State,
        done: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
        cancel: Arc<AtomicBool>,
        /// The status line has been told how this ended.
        announced: bool,
        /// A program to run at the start or end of the job, from the Advanced Settings.
        script: Option<(String, bool)>,
    }

    /// What the page shows of an item.
    struct Row {
        id: u64,
        name: String,
        preset: Preset,
        state: State,
        fraction: f32,
    }

    /// Which part of the timeline to render.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Range {
        Whole,
        Marks,
    }

    pub(crate) struct Deliver {
        queue: Arc<Mutex<Vec<Item>>>,
        running: Arc<AtomicBool>,
        next_id: u64,
        pub(crate) preset: Preset,
        pub(crate) range: Range,
        /// What the Deliver page sets besides the preset (the size and rate come from the page).
        pub(crate) options: Options,
        /// The preset whose output the user is choosing in a dialog.
        pending: Option<Preset>,
    }

    impl Default for Deliver {
        fn default() -> Self {
            Self {
                queue: Arc::default(),
                running: Arc::default(),
                next_id: 1,
                preset: Preset::H264Web,
                range: Range::Whole,
                options: Options::default(),
                pending: None,
            }
        }
    }

    fn lock(queue: &Mutex<Vec<Item>>) -> MutexGuard<'_, Vec<Item>> {
        queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl Deliver {
        /// Tells the status line how finished exports ended; keeps the window redrawing while
        /// one runs.
        pub(crate) fn poll(&mut self, status: &mut Status, ctx: &Context) {
            let mut queue = lock(&self.queue);
            for item in queue.iter_mut().filter(|i| !i.announced) {
                let text = match &item.state {
                    State::Done(took) => {
                        format!("Rendered {} in {:.1} s", item.name, took.as_secs_f32())
                    }
                    State::Failed(why) => format!("Rendering {} failed: {why}", item.name),
                    State::Cancelled => format!("Rendering {} was cancelled", item.name),
                    State::Queued | State::Running => continue,
                };
                item.announced = true;
                *status = Status {
                    text,
                    error: matches!(item.state, State::Failed(_)),
                };
            }
            if queue.iter().any(|i| i.state == State::Running) {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        fn rows(&self) -> Vec<Row> {
            lock(&self.queue)
                .iter()
                .map(|i| Row {
                    id: i.id,
                    name: i.name.clone(),
                    preset: i.settings.preset,
                    state: i.state.clone(),
                    fraction: i.done.load(Ordering::Relaxed) as f32
                        / i.total.load(Ordering::Relaxed).max(1) as f32,
                })
                .collect()
        }

        /// The state of every item, in order.
        #[cfg(test)]
        pub(crate) fn states(&self) -> Vec<State> {
            lock(&self.queue).iter().map(|i| i.state.clone()).collect()
        }
    }

    /// Starts the program the Advanced Settings name, with `start` or `end` and the output
    /// path as its arguments, when it is for this moment of the job. A program that cannot be
    /// started is left alone: it must not stop the render.
    fn run_script(script: &Option<(String, bool)>, at_end: bool, output: &std::path::Path) {
        let Some((program, wanted_at_end)) = script else {
            return;
        };
        if *wanted_at_end != at_end {
            return;
        }
        let _ = std::process::Command::new(program)
            .arg(if at_end { "end" } else { "start" })
            .arg(output)
            .spawn();
    }

    /// Runs `f` under `std::panic::catch_unwind` so a panic in the render worker never
    /// propagates and kills the queue thread. Returns `Err` with a fixed message if `f`
    /// panicked. Takes a plain closure so it can be tested without FFmpeg.
    fn catch_render_panic(
        f: impl FnOnce() -> Result<(), ExportError>,
    ) -> Result<(), ExportError> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(result) => result,
            Err(_) => Err(ExportError::Render("the render stopped unexpectedly".into())),
        }
    }

    /// The thread that renders the queue: takes the first waiting item, renders it, and goes on
    /// until nothing waits.
    fn run_worker(queue: &Mutex<Vec<Item>>, running: &AtomicBool, ctx: &Context) {
        loop {
            let job = {
                let mut q = lock(queue);
                q.iter_mut().find(|i| i.state == State::Queued).map(|i| {
                    i.state = State::Running;
                    (
                        i.id,
                        i.project.clone(),
                        i.cache.clone(),
                        i.settings.clone(),
                        (i.done.clone(), i.total.clone(), i.cancel.clone()),
                        i.script.clone(),
                    )
                })
            };
            let Some((id, project, cache, settings, (done, total, cancel), script)) = job else {
                running.store(false, Ordering::SeqCst);
                // Something may have been added just as the worker was leaving.
                let waiting = lock(queue).iter().any(|i| i.state == State::Queued);
                if waiting
                    && running
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                {
                    continue;
                }
                return;
            };
            let started = Instant::now();
            run_script(&script, false, &settings.path);
            let result = catch_render_panic(|| {
                render(&project, &cache, &settings, &mut |d, t| {
                    done.store(d, Ordering::Relaxed);
                    total.store(t, Ordering::Relaxed);
                    ctx.request_repaint();
                    !cancel.load(Ordering::Relaxed)
                })
            });
            if result.is_ok() {
                run_script(&script, true, &settings.path);
            } else if !settings.preset.is_folder() {
                // Same cleanup render() does for its own errors; needed here because a panicked
                // render never reaches its own cleanup.
                let _ = std::fs::remove_file(&settings.path);
            }
            if result.is_err() {
                warn!("render job {} stopped unexpectedly", id);
            }
            if let Some(item) = lock(queue).iter_mut().find(|i| i.id == id) {
                item.state = match result {
                    Ok(()) => State::Done(started.elapsed()),
                    Err(ExportError::Cancelled) => State::Cancelled,
                    Err(e) => State::Failed(e.to_string()),
                };
            }
            ctx.request_repaint();
        }
    }

    /// How many steps a render takes: the pictures of the file.
    fn options_total(project: &Project, start: u64, end: u64, options: &Options) -> u64 {
        options.output_frames(project, end - start).max(1)
    }

    impl StudioApp {
        /// The user chose where the pending preset's output goes: adds it to the queue, with the
        /// project as it is now.
        pub(crate) fn enqueue_export(&mut self, path: String) {
            // A folder chosen with Browse sets the location and nothing more.
            if std::mem::take(&mut self.deliver_ui.browsing) {
                self.deliver_ui.location = path;
                return;
            }
            let Some(preset) = self.deliver.pending.take() else {
                return;
            };
            let project = self.project();
            let length = project.duration();
            let (start, end) = match self.deliver.range {
                Range::Whole => (0, length),
                Range::Marks => {
                    let start = project.mark_in.unwrap_or(0).min(length);
                    (
                        start,
                        project.mark_out.unwrap_or(length).min(length).max(start),
                    )
                }
            };
            self.enqueue_job(preset, path, start, end);
        }

        /// Writes the titles between `start` and `end` as a subtitle file next to `path`, when
        /// the Subtitle Settings ask for a separate file.
        fn write_subtitles(&mut self, path: &str, start: u64, end: u64) {
            if !self.deliver_ui.export_subtitle || self.deliver_ui.subtitle_format != 0 {
                return;
            }
            let kind = self.deliver_ui.subtitle_kind;
            let cues = crate::subtitles::cues(self.project(), start, end);
            if cues.is_empty() {
                self.info("No subtitles written: there are no titles in the range");
                return;
            }
            let text = crate::subtitles::write(kind, &cues, self.project().fps);
            let file = PathBuf::from(path).with_extension(kind.extension());
            match std::fs::write(&file, text) {
                Ok(()) => self.info(format!(
                    "Wrote {} subtitle(s) to {}",
                    cues.len(),
                    file.display()
                )),
                Err(e) => self.fail(format!("could not write {}: {e}", file.display())),
            }
        }

        /// Puts one render in the queue, with the project and the options as they are now.
        fn enqueue_job(&mut self, preset: Preset, path: String, start: u64, end: u64) {
            if end <= start {
                self.fail("there is nothing to render in that range");
                return;
            }
            if !preset.is_folder() {
                self.write_subtitles(&path, start, end);
            }
            let options = self.export_options();
            let options_for_total = options.clone();
            let project = self.project();
            let name = std::path::Path::new(&path)
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            let total = options_total(project, start, end, &options_for_total);
            let item = Item {
                id: self.deliver.next_id,
                name: name.clone(),
                settings: Settings {
                    preset,
                    path: PathBuf::from(path),
                    start,
                    end,
                    options,
                },
                project: Arc::new(project.clone()),
                // The originals, unless the Deliver page was told to use the optimised copies.
                cache: self.export_cache(),
                state: State::Queued,
                done: Arc::default(),
                total: Arc::new(AtomicU64::new(total)),
                cancel: Arc::default(),
                announced: false,
                script: self
                    .deliver_ui
                    .script_on
                    .then(|| {
                        (
                            self.deliver_ui.script_path.trim().to_string(),
                            self.deliver_ui.script_at_end,
                        )
                    })
                    .filter(|(path, _)| !path.is_empty()),
            };
            self.deliver.next_id += 1;
            lock(&self.deliver.queue).push(item);
            self.info(format!("Added {name} to the render queue"));
        }

        /// Adds a render of the whole timeline (or the marked part) to the queue at once, without
        /// asking where to put it.
        #[cfg(test)]
        pub(crate) fn queue_render(&mut self, preset: Preset, path: String) {
            self.deliver.pending = Some(preset);
            self.enqueue_export(path);
        }

        /// Starts rendering what waits in the queue (does nothing if it is already running).
        pub(crate) fn start_queue(&mut self, ctx: &Context) {
            let d = &self.deliver;
            if d.running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return;
            }
            let (queue, running, ctx) = (d.queue.clone(), d.running.clone(), ctx.clone());
            std::thread::spawn(move || run_worker(&queue, &running, &ctx));
        }

        /// The jobs of the render queue, in order.
        pub(crate) fn jobs(&self) -> Vec<super::Job> {
            self.deliver
                .rows()
                .into_iter()
                .map(|r| super::Job {
                    id: r.id,
                    name: r.name,
                    label: r.preset.label().to_string(),
                    state: match r.state {
                        State::Queued => super::JobState::Queued,
                        State::Running => super::JobState::Running,
                        State::Done(took) => super::JobState::Done(took.as_secs_f32()),
                        State::Failed(why) => super::JobState::Failed(why),
                        State::Cancelled => super::JobState::Cancelled,
                    },
                    fraction: r.fraction,
                })
                .collect()
        }

        pub(crate) fn cancel_job(&mut self, id: u64) {
            if let Some(item) = lock(&self.deliver.queue).iter_mut().find(|i| i.id == id) {
                item.cancel.store(true, Ordering::Relaxed);
                if item.state == State::Queued {
                    item.state = State::Cancelled;
                }
            }
        }

        pub(crate) fn remove_job(&mut self, id: u64) {
            lock(&self.deliver.queue).retain(|i| i.id != id || i.state == State::Running);
        }

        pub(crate) fn clear_finished_jobs(&mut self) {
            lock(&self.deliver.queue).retain(|i| matches!(i.state, State::Queued | State::Running));
        }

        /// Starts rendering what waits in the queue.
        pub(crate) fn render_all(&mut self, ctx: &Context) {
            self.start_queue(ctx);
        }

        pub(crate) fn render_busy(&self) -> bool {
            self.deliver.running.load(Ordering::SeqCst)
        }

        /// Whether this build can render at all.
        pub(crate) fn can_render_here(&self) -> bool {
            true
        }

        /// The options the page has set, with the size, frame rate and key frames it works out.
        pub(crate) fn export_options(&self) -> Options {
            let ui = &self.deliver_ui;
            let project = self.project();
            let mut o = self.deliver.options.clone();
            o.size = match ui.res_mode {
                0 => None,
                n if n <= crate::deliver::RESOLUTIONS.len() => crate::deliver::RESOLUTIONS
                    .get(n - 1)
                    .map(|(_, w, h)| (*w, *h)),
                _ => {
                    let (w, h) = ui.custom;
                    if ui.vertical {
                        // The vertical resolution is the height; the width keeps the timeline's shape.
                        let aspect = project.width as f32 / project.height.max(1) as f32;
                        Some(((h as f32 * aspect).round() as u32, h))
                    } else {
                        Some((w, h))
                    }
                }
            };
            o.fps = match ui.fps_choice {
                0 => None,
                n => picasso_core::Fps::PRESETS.get(n - 1).copied(),
            };
            o.keyframes = if ui.key_auto {
                None
            } else {
                Some(ui.every.max(1))
            };
            o.pixel_aspect = if ui.cinemascope {
                picasso_export::PixelAspect::Cinemascope
            } else {
                picasso_export::PixelAspect::Square
            };
            o.levels = match ui.levels {
                1 => picasso_export::Levels::Video,
                2 => picasso_export::Levels::Full,
                _ => picasso_export::Levels::Auto,
            };
            o.color_primaries = crate::deliver::COLOR_TAGS
                .get(ui.color_tag)
                .and_then(|t| t.1);
            o.color_trc = crate::deliver::GAMMA_TAGS
                .get(ui.gamma_tag)
                .and_then(|t| t.1);
            o.burn_in = match ui.burn {
                2 => picasso_export::BurnIn::Timecode,
                3 => picasso_export::BurnIn::ClipName,
                4 => picasso_export::BurnIn::Both,
                _ => picasso_export::BurnIn::Off,
            };
            // The timeline's clock starts at 01:00:00:00, as the page shows it.
            o.clock_start = 3600 * project.fps.nominal();
            o
        }

        pub(crate) fn range_is_whole(&self) -> bool {
            self.deliver.range == Range::Whole
        }

        pub(crate) fn set_range_whole(&mut self, whole: bool) {
            self.deliver.range = if whole { Range::Whole } else { Range::Marks };
        }

        /// A rough size of the file the settings would give, in megabytes.
        pub(crate) fn estimated_mb(&self) -> f32 {
            let p = self.project();
            let o = self.export_options();
            let (frames, fps) = match self.deliver.range {
                Range::Whole => (p.duration(), o.frame_rate(p)),
                Range::Marks => {
                    let start = p.mark_in.unwrap_or(0);
                    let end = p.mark_out.unwrap_or(p.duration());
                    (end.saturating_sub(start), o.frame_rate(p))
                }
            };
            let seconds = frames as f64 / p.fps.as_f64().max(1.0);
            let (w, h) = o.picture_size(p);
            let scale = f64::from(w) * f64::from(h) / (1920.0 * 1080.0);
            let speed = fps.as_f64() / 24.0;
            let video_mbps = match (self.deliver.preset, o.rate) {
                (
                    Preset::H264Web | Preset::H265 | Preset::Av1 | Preset::Vp9,
                    picasso_export::Rate::Bitrate(kbps),
                ) => f64::from(kbps) / 1000.0,
                (Preset::Av1, picasso_export::Rate::Quality(q)) => {
                    scale * speed * super::quality_mbps(q) * 0.5
                }
                (Preset::Vp9, picasso_export::Rate::Quality(q)) => {
                    scale * speed * super::quality_mbps(q) * 0.65
                }
                // Avid's rates at 1080p, 24 frames a second.
                (Preset::DnxHr, _) => {
                    let rates = [36.0, 116.0, 176.0, 176.0, 352.0];
                    scale * speed * rates[usize::from(o.dnxhr).min(rates.len() - 1)]
                }
                // Lossless: about a third of the raw 10-bit picture.
                (Preset::Ffv1, _) => scale * speed * 450.0,
                (Preset::Mp3, _) => 0.0,
                (Preset::H264Web, picasso_export::Rate::Quality(q)) => {
                    scale * speed * super::quality_mbps(q)
                }
                (Preset::H265, picasso_export::Rate::Quality(q)) => {
                    scale * speed * super::quality_mbps(q) * 0.6
                }
                (Preset::ProRes422, _) => {
                    scale * speed * [45.0, 102.0, 147.0, 220.0][usize::from(o.prores.min(3))]
                }
                (Preset::PngSequence, _) => 40.0 * fps.as_f64() * scale,
                (Preset::Wav, _) => 0.0,
            };
            let video = if o.video { video_mbps } else { 0.0 };
            let audio = if !o.audio {
                0.0
            } else if self.deliver.preset == Preset::Wav {
                48_000.0 * 24.0 * 2.0 / 1.0e6
            } else if matches!(self.deliver.preset, Preset::ProRes422 | Preset::DnxHr)
                && o.audio_codec != picasso_export::AudioCodec::Aac
            {
                48_000.0 * 16.0 * 2.0 / 1.0e6
            } else if self.deliver.preset == Preset::Ffv1 {
                // FLAC: about 60 % of 16-bit PCM.
                48_000.0 * 16.0 * 2.0 * 0.6 / 1.0e6
            } else {
                f64::from(o.audio_kbps) / 1000.0
            };
            (seconds * (video + audio) / 8.0) as f32
        }

        /// Adds a render of what the Deliver page is set to: into the location and file name
        /// given there, or into a place the user is asked for. With "Individual clips" every
        /// clip of the video tracks is a render of its own.
        pub(crate) fn add_to_queue(&mut self, ctx: &Context) {
            let preset = self.deliver.preset;
            if self.project().duration() == 0 {
                return;
            }
            let name = self.deliver_ui.file_name.trim().to_string();
            let name = if name.is_empty() {
                self.project().name.clone()
            } else {
                name
            };
            let location = self.deliver_ui.location.trim().to_string();
            if self.deliver_ui.individual && !preset.is_folder() {
                if location.is_empty() {
                    self.fail("choose a Location to render the clips into");
                    return;
                }
                let mut clips: Vec<(u64, u64)> = self
                    .project()
                    .tracks
                    .iter()
                    .filter(|t| t.kind == picasso_core::TrackKind::Video)
                    .flat_map(|t| t.clips.iter().map(|c| (c.start, c.start + c.duration)))
                    .collect();
                clips.sort_unstable();
                clips.dedup();
                if clips.is_empty() {
                    self.fail("there are no clips to render");
                    return;
                }
                for (n, (start, end)) in clips.into_iter().enumerate() {
                    let file = format!("{name}_{:02}", n + 1);
                    let path = self.output_path(&location, &file, preset);
                    self.deliver.pending = Some(preset);
                    self.enqueue_job(preset, path, start, end);
                }
                self.deliver.pending = None;
                return;
            }
            self.deliver.pending = Some(preset);
            if !location.is_empty() {
                let path = if preset.is_folder() {
                    location
                } else {
                    self.output_path(&location, &name, preset)
                };
                self.enqueue_export(path);
            } else if preset.is_folder() {
                platform::pick_folder(self.inbox.clone(), ctx.clone());
            } else {
                platform::pick_export_file(
                    self.inbox.clone(),
                    ctx.clone(),
                    format!("{name}.{}", preset.extension()),
                    preset.extension(),
                );
            }
        }

        /// The file for `name` in `location`; with "Use unique filename", one that is not there
        /// yet (name (1), name (2), ...).
        fn output_path(&self, location: &str, name: &str, preset: Preset) -> String {
            let ext = preset.extension();
            let mut path = PathBuf::from(location);
            path.push(format!("{name}.{ext}"));
            if self.deliver_ui.unique {
                let mut n = 1;
                while path.exists() || self.queued_path(&path) {
                    path = PathBuf::from(location);
                    path.push(format!("{name} ({n}).{ext}"));
                    n += 1;
                    if n > 9_999 {
                        break;
                    }
                }
            }
            path.to_string_lossy().into_owned()
        }

        /// Whether a job in the queue already renders into `path`.
        fn queued_path(&self, path: &std::path::Path) -> bool {
            lock(&self.deliver.queue).iter().any(|i| {
                i.settings.path == path && !matches!(i.state, State::Failed(_) | State::Cancelled)
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Without the guard, a panic inside the closure propagates out of `catch_unwind` and
        /// would kill the worker thread.
        #[test]
        fn an_unguarded_panic_propagates_out_of_catch_unwind() {
            let unguarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                panic!("render panicked");
            }));
            assert!(
                unguarded.is_err(),
                "an unguarded panic must propagate as Err"
            );
        }

        /// With the guard, a panic inside the closure is caught and returned as
        /// `ExportError::Render("the render stopped unexpectedly")` instead of unwinding.
        #[test]
        fn catch_render_panic_catches_a_panic_and_returns_the_unexpected_message() {
            let result = catch_render_panic(|| panic!("render panicked"));
            assert!(
                matches!(result, Err(ExportError::Render(ref text)) if text == "the render stopped unexpectedly"),
                "expected Err(Render(\"the render stopped unexpectedly\")), got {result:?}"
            );
        }

        /// The guard passes through both Ok and non-panic Err results unchanged.
        #[test]
        fn catch_render_panic_passes_through_normal_results() {
            assert!(catch_render_panic(|| Ok(())).is_ok());
            assert!(matches!(
                catch_render_panic(|| Err(ExportError::Cancelled)),
                Err(ExportError::Cancelled)
            ));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn quality_mbps(q: picasso_export::Quality) -> f64 {
    match q {
        picasso_export::Quality::Best => 20.0,
        picasso_export::Quality::High => 10.0,
        picasso_export::Quality::Medium => 5.0,
        picasso_export::Quality::Low => 2.5,
    }
}
