//! Optimised media: small copies of video files that decode quickly, for smooth editing.
//!
//! A 4K file with few key frames takes seconds to show an exact picture. A copy at 1280×720
//! with a key frame every few pictures takes milliseconds, so the timeline and the viewer stay
//! smooth. The original is still what an export reads, and what sound is read from.
//!
//! The copies live in the data folder, not beside a project: one copy of a file serves every
//! project that uses it, and a project file never points at a copy. An index beside them says
//! which file each copy was made from, with that file's size and date, so a copy of a file that
//! has changed since is not used.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::app::StudioApp;

/// The longest side of an optimised copy, in pixels: a 4K and a 1080p file both become
/// 1280×720, and a file already smaller than that keeps its size.
pub(crate) const LONG_SIDE: u32 = 1280;

/// The index file, beside the copies.
const INDEX: &str = "index.txt";

/// What is known of one copy.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    /// The copy's file name in the folder.
    file: String,
    /// The size and date of the file it was made from: when either differs, the copy is stale.
    size: u64,
    modified: u64,
}

/// The copies that were made, by the file they were made from.
#[derive(Clone, Debug, Default)]
pub(crate) struct Store {
    /// Where the copies and the index are. `None` when there is no data folder (the browser).
    root: Option<PathBuf>,
    entries: HashMap<String, Entry>,
    /// Counts the changes, so the pictures are put together again when a copy appears or goes.
    revision: u64,
}

impl Store {
    /// The store in the data folder, as it is on disk.
    pub(crate) fn load() -> Store {
        let root = crate::project_manager::data_dir().map(|d| d.join("optimised"));
        let store = Self::load_from(root);
        store.sweep();
        store
    }

    /// Removes the files of copies that were never finished: a copy that was being made when the
    /// program was closed leaves a file the index does not name. Only files an hour old or more go,
    /// so a copy another running program is still making is left alone.
    pub(crate) fn sweep(&self) -> usize {
        let Some(root) = &self.root else {
            return 0;
        };
        let Ok(dir) = std::fs::read_dir(root) else {
            return 0;
        };
        let known: std::collections::HashSet<&str> =
            self.entries.values().map(|e| e.file.as_str()).collect();
        let hour = std::time::Duration::from_secs(3600);
        let mut gone = 0;
        for item in dir.flatten() {
            let path = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            if path.extension().is_none_or(|e| e != "mp4") || known.contains(name.as_str()) {
                continue;
            }
            let old = item
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age >= hour);
            if old && std::fs::remove_file(&path).is_ok() {
                gone += 1;
            }
        }
        gone
    }

    /// The store kept under `root` (a test says where).
    pub(crate) fn load_from(root: Option<PathBuf>) -> Store {
        let mut store = Store {
            root,
            entries: HashMap::new(),
            revision: 0,
        };
        let Some(text) = store.index_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return store;
        };
        for line in text.lines() {
            // path<TAB>file<TAB>size<TAB>modified
            let mut parts = line.split('\t');
            let (Some(source), Some(file), Some(size), Some(modified)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let (Ok(size), Ok(modified)) = (size.parse(), modified.parse()) else {
                continue;
            };
            // The index is a file anyone can edit: a copy is a plain file name in the folder, never a
            // path out of it (deleting copies would delete whatever the path names).
            if source.is_empty()
                || file.is_empty()
                || Path::new(file).file_name() != Some(std::ffi::OsStr::new(file))
            {
                continue;
            }
            store.entries.insert(
                source.to_string(),
                Entry {
                    file: file.to_string(),
                    size,
                    modified,
                },
            );
        }
        store
    }

    fn index_path(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(INDEX))
    }

    /// Where a copy of `source` would be kept: in the folder, named after the file and a hash of
    /// its whole path, so two files of one name in different folders do not share a copy.
    pub(crate) fn place_for(&self, source: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        // The index has a line for each file: a name with a tab or a line break in it cannot be
        // written there, so such a file keeps no copy.
        if source.contains(['\t', '\n', '\r']) {
            return None;
        }
        let stem: String = Path::new(source)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "media".into());
        let stem: String = stem
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .take(60)
            .collect();
        let stem = stem.trim_matches('_');
        let stem = if stem.is_empty() { "media" } else { stem };
        // Two paths with one hash (rare, but then the wrong picture would show, silently) never
        // share a name: the next free one is taken.
        let mut name = format!("{stem}-{:016x}.mp4", hash(source));
        let mut n = 2;
        while self.entries.iter().any(|(other, e)| e.file == name && other != source) {
            name = format!("{stem}-{:016x}-{n}.mp4", hash(source));
            n += 1;
        }
        Some(root.join(name))
    }

    /// The copy to decode for `source`, when there is one and the file has not changed since.
    pub(crate) fn path_for(&self, source: &str) -> Option<PathBuf> {
        let entry = self.entries.get(source)?;
        let file = self.root.as_ref()?.join(&entry.file);
        if !file.is_file() {
            return None;
        }
        let (size, modified) = stamp(Path::new(source))?;
        (size == entry.size && modified == entry.modified).then_some(file)
    }

    /// Says that `file` is now the copy of `source`.
    pub(crate) fn record(&mut self, source: &str, file: &Path) {
        let Some((size, modified)) = stamp(Path::new(source)) else {
            return;
        };
        let Some(name) = file.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            return;
        };
        self.entries.insert(
            source.to_string(),
            Entry {
                file: name,
                size,
                modified,
            },
        );
        self.revision += 1;
    }

    /// Forgets the copy of `source`, deleting the file too.
    pub(crate) fn forget(&mut self, source: &str) {
        if let Some(entry) = self.entries.remove(source) {
            if let Some(root) = &self.root {
                let _ = std::fs::remove_file(root.join(entry.file));
            }
            self.revision += 1;
        }
    }

    /// Deletes every copy and the index.
    pub(crate) fn clear(&mut self) {
        let root = self.root.clone();
        for entry in self.entries.values() {
            if let Some(root) = &root {
                let _ = std::fs::remove_file(root.join(&entry.file));
            }
        }
        self.entries.clear();
        self.revision += 1;
        if let Some(index) = self.index_path() {
            let _ = std::fs::remove_file(index);
        }
    }

    /// Deletes the copies of files that are gone, and copies whose file has changed since.
    /// Returns how many went.
    pub(crate) fn prune(&mut self) -> usize {
        let stale: Vec<String> = self
            .entries
            .iter()
            .filter(|(source, entry)| match stamp(Path::new(source)) {
                Some((size, modified)) => size != entry.size || modified != entry.modified,
                // A file on a drive that is not plugged in is not gone: only a file that is
                // nowhere is dropped, and that is what `stamp` cannot tell apart. Kept.
                None => false,
            })
            .map(|(source, _)| source.clone())
            .collect();
        for source in &stale {
            self.forget(source);
        }
        stale.len()
    }

    /// Every copy, by the file it was made from.
    pub(crate) fn copies(&self) -> Vec<(String, PathBuf)> {
        let Some(root) = &self.root else {
            return Vec::new();
        };
        self.entries
            .iter()
            .map(|(source, entry)| (source.clone(), root.join(&entry.file)))
            .collect()
    }

    /// Changes when a copy is added or removed, so what is drawn from them is drawn again.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether there is anywhere to keep copies: the browser has no data folder.
    pub(crate) fn has_folder(&self) -> bool {
        self.root.is_some()
    }

    /// Where the copies are kept, for the window to say.
    pub(crate) fn folder(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Writes the index, so the next start knows what is there. Written beside it first, so a
    /// crash while writing never leaves half an index.
    pub(crate) fn save(&self) {
        let (Some(root), Some(index)) = (self.root.as_ref(), self.index_path()) else {
            return;
        };
        if std::fs::create_dir_all(root).is_err() {
            return;
        }
        let mut text = String::new();
        let mut rows: Vec<_> = self.entries.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        for (source, entry) in rows {
            text.push_str(&format!(
                "{source}\t{}\t{}\t{}\n",
                entry.file, entry.size, entry.modified
            ));
        }
        let tmp = index.with_extension("txt.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &index);
        }
    }
}

/// A file's size and the second it was last written.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some((meta.len(), modified))
}

/// FNV-1a of the path's bytes: short, stable, and no dependency.
fn hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Makes the copy of `source` at `dest`: the file rendered again, smaller, with a key frame
/// every few pictures so a picture can be found without reading the ones before it.
///
/// It is the export pipeline on a project of one clip, so the copy has the file's own frame
/// rate and length and the frames line up with the original's: the same time asks for the same
/// picture. Runs on a thread of its own; `progress` gets the frames done and the total.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn make(
    source: &Path,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(), String> {
    let info = picasso_decode::probe(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let (num, den) = info.fps;
    if num == 0 || den == 0 {
        return Err(format!("{}: the file does not say its frame rate", source.display()));
    }
    if info.variable_frame_rate {
        return Err(VARIABLE_RATE.to_string());
    }
    let fps = picasso_core::Fps { num, den };
    let duration_us = info
        .duration_us
        .filter(|d| *d > 0)
        .ok_or_else(|| format!("{}: the file does not say how long it is", source.display()))?;
    let frames = (u128::try_from(duration_us).unwrap_or(0) * u128::from(num)
        / (1_000_000 * u128::from(den))) as u64;
    if frames == 0 {
        return Err(format!("{}: the file has no pictures", source.display()));
    }

    // A project of only this file, at the file's own size and frame rate.
    let mut project = picasso_core::Project::new("Optimised media");
    project.fps = fps;
    project.width = info.width.max(2);
    project.height = info.height.max(2);
    let mut session = picasso_core::Session::new(project);
    let name = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".into());
    let media = match session
        .dispatch(picasso_core::Command::AddMedia {
            name,
            source: picasso_core::MediaSource::Video {
                path: source.to_string_lossy().into_owned(),
            },
            length: Some(frames),
            default_duration: frames,
        })
        .map_err(|e| e.to_string())?
    {
        picasso_core::Outcome::Media(id) => id,
        other => return Err(format!("the file was not added: {other:?}")),
    };
    let track = match session
        .dispatch(picasso_core::Command::AddTrack {
            kind: picasso_core::TrackKind::Video,
        })
        .map_err(|e| e.to_string())?
    {
        picasso_core::Outcome::Track(id) => id,
        other => return Err(format!("the track was not added: {other:?}")),
    };
    session
        .dispatch(picasso_core::Command::AddClip {
            track,
            media,
            start: 0,
            duration: Some(frames),
            with_sound: false,
        })
        .map_err(|e| e.to_string())?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let cache = crate::video::new_cache(&egui::Context::default());
    let settings = picasso_export::Settings {
        preset: picasso_export::Preset::H264Web,
        path: dest.to_path_buf(),
        start: 0,
        end: frames,
        options: picasso_export::Options {
            // The sound stays with the original; a copy is only for the picture.
            audio: false,
            size: Some(copy_size((info.width, info.height))),
            fps: Some(fps),
            // A key frame often enough that a picture is found without reading back far, and no
            // pictures that refer to later ones, which is what makes scrubbing slow.
            keyframes: Some(8),
            frame_reordering: false,
            rate: picasso_export::Rate::Quality(picasso_export::Quality::Medium),
            ..picasso_export::Options::default()
        },
    };
    picasso_export::render(session.project(), &cache, &settings, progress)
        .map_err(|e| format!("{}: {e}", dest.display()))
}

/// The size a copy of a `(width, height)` picture has: inside [`LONG_SIDE`] on its longest
/// side, the same shape, and never bigger than the file itself.
pub(crate) fn copy_size((width, height): (u32, u32)) -> (u32, u32) {
    let longest = width.max(height);
    if longest == 0 || longest <= LONG_SIDE {
        return (width.max(2), height.max(2));
    }
    let scale = f64::from(LONG_SIDE) / f64::from(longest);
    let w = ((f64::from(width) * scale).round() as u32).max(2) & !1;
    let h = ((f64::from(height) * scale).round() as u32).max(2) & !1;
    (w, h)
}

/// The copies being made: one at a time, so two of them do not fight over the processor and the
/// window says clearly how far the work has got.
/// What a copy that was stopped reports, so it is not shown as a failure.
pub(crate) const STOPPED: &str = "stopped";
/// Why a file keeps no copy: its pictures are not a fixed time apart.
pub(crate) const VARIABLE_RATE: &str = "its frame rate is not fixed, so a copy would not have its pictures at the same times";

#[derive(Clone, Debug, Default)]
pub(crate) struct Jobs {
    /// Set to stop the copy being made; the thread making it looks at it with every picture.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Files that cannot have a copy (this run), so they are not asked for again.
    unsuitable: std::collections::HashSet<String>,
    /// The files waiting, in the order they were asked for.
    queue: std::collections::VecDeque<String>,
    /// The file being made now, with the pictures done and the total.
    running: Option<(String, u64, u64)>,
}

impl Jobs {
    /// Whether `source` is waiting or being made now.
    pub(crate) fn holds(&self, source: &str) -> bool {
        self.queue.iter().any(|s| s == source)
            || self.running.as_ref().is_some_and(|(s, _, _)| s == source)
    }

    /// Puts `source` in line. False when it was already there.
    pub(crate) fn enqueue(&mut self, source: &str) -> bool {
        if self.holds(source) {
            return false;
        }
        self.queue.push_back(source.to_string());
        true
    }

    /// The next file to make, now the one being made. None while one is being made: the copies
    /// are made one at a time.
    pub(crate) fn take(&mut self) -> Option<String> {
        if self.running.is_some() {
            return None;
        }
        let source = self.queue.pop_front()?;
        self.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        self.running = Some((source.clone(), 0, 0));
        Some(source)
    }

    /// Says how far the copy of `source` has got, in pictures.
    pub(crate) fn progress(&mut self, source: &str, done: u64, total: u64) {
        if let Some((s, d, t)) = &mut self.running {
            if s == source {
                (*d, *t) = (done, total);
            }
        }
    }

    /// The copy of `source` was made, or could not be: it is no longer being made.
    pub(crate) fn done(&mut self, source: &str) {
        if self.running.as_ref().is_some_and(|(s, _, _)| s == source) {
            self.running = None;
        }
        self.queue.retain(|s| s != source);
    }

    /// How far the copy of `source` is, from 0 to 1, while it is being made.
    pub(crate) fn fraction(&self, source: &str) -> Option<f32> {
        let (s, done, total) = self.running.as_ref()?;
        if s != source {
            return None;
        }
        Some(if *total == 0 {
            0.0
        } else {
            (*done as f32 / *total as f32).clamp(0.0, 1.0)
        })
    }

    /// Stops the copy being made and forgets the ones waiting. The copies already made stay.
    pub(crate) fn cancel_all(&mut self) {
        self.queue.clear();
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The flag the thread making a copy looks at.
    pub(crate) fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.cancel.clone()
    }

    /// Says that `source` cannot have a copy.
    pub(crate) fn mark_unsuitable(&mut self, source: &str) {
        self.unsuitable.insert(source.to_string());
    }

    /// Whether `source` was found unable to have a copy.
    pub(crate) fn is_unsuitable(&self, source: &str) -> bool {
        self.unsuitable.contains(source)
    }

    /// Whether anything is waiting or being made.
    pub(crate) fn busy(&self) -> bool {
        self.running.is_some() || !self.queue.is_empty()
    }
}

/// Makes the copy of `source` in `dest` on a thread of its own, saying how far it has got and
/// how it ended. The inbox brings both back to the window.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn start(
    source: String,
    dest: PathBuf,
    inbox: crate::platform::Inbox,
    ctx: egui::Context,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut told = 0u64;
        let result = make(Path::new(&source), &dest, &mut |done, total| {
            // Every handful of pictures, so a long file says it is working without a message
            // for every one of its pictures.
            if done == total || done.saturating_sub(told) >= 8 {
                told = done;
                inbox.push(
                    &ctx,
                    crate::platform::Incoming::ProxyProgress {
                        source: source.clone(),
                        done,
                        total,
                    },
                );
            }
            !cancel.load(std::sync::atomic::Ordering::Relaxed)
        });
        let result = match result {
            Err(_) if cancel.load(std::sync::atomic::Ordering::Relaxed) => Err(STOPPED.to_string()),
            other => other,
        };
        // A failed render leaves a truncated file behind; remove it so a retry makes a fresh one
        // and no orphan is left in the data folder.
        let made = match result {
            Ok(()) => Ok(dest.to_string_lossy().into_owned()),
            Err(e) => {
                let _ = std::fs::remove_file(&dest);
                Err(e)
            }
        };
        inbox.push(
            &ctx,
            crate::platform::Incoming::ProxyDone {
                source,
                made,
            },
        );
    });
}

/// The file's name as the window says it.
pub(crate) fn name_of(source: &str) -> String {
    Path::new(source)
        .file_name()
        .map_or_else(|| source.to_string(), |n| n.to_string_lossy().into_owned())
}

/// A number that changes when the copies the viewer reads would change: the switch, the copies
/// that were made, and the video files the project uses.
fn signature<'a>(
    use_optimised: bool,
    revision: u64,
    paths: impl Iterator<Item = &'a str>,
) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    add(&[u8::from(use_optimised)]);
    add(&revision.to_le_bytes());
    for path in paths {
        add(path.as_bytes());
        add(&[0]);
    }
    h
}

impl StudioApp {
    /// The video files in the media pool, used on a timeline or not, as they are on disk. A file
    /// imported twice is listed once. This is what Media Management lists, makes copies of and
    /// deletes copies of, as the viewer reads a copy for any video of the pool.
    pub(crate) fn pool_videos(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.project()
            .media
            .iter()
            .filter_map(|m| match &m.source {
                picasso_core::MediaSource::Video { path } => Some(path.clone()),
                _ => None,
            })
            .filter(|path| seen.insert(path.clone()))
            .collect()
    }

    /// Works out which files the viewer should read copies of, and gives them to the cache.
    /// Called when the copies, the media or the switch change, not every frame.
    pub(crate) fn refresh_proxies(&mut self) {
        let mut copies = HashMap::new();
        if self.use_optimised {
            // Every video in the pool, used or not: the media pool's thumbnails are small, and
            // a 4K picture per tile adds up.
            for item in &self.project().media {
                if let picasso_core::MediaSource::Video { path } = &item.source {
                    if let Some(copy) = self.proxies.path_for(path) {
                        copies.insert(path.clone(), copy.to_string_lossy().into_owned());
                    }
                }
            }
        }
        self.cache = self.cache.with_copies(copies);
        self.proxy_stamp = self.proxy_signature();
        // The pictures on screen were put together from other files.
        self.media_generation += 1;
    }

    fn proxy_signature(&self) -> u64 {
        let paths = self.project().media.iter().filter_map(|m| match &m.source {
            picasso_core::MediaSource::Video { path } => Some(path.as_str()),
            _ => None,
        });
        signature(self.use_optimised, self.proxies.revision(), paths)
    }

    /// Refreshes the copies when they, the media or the switch have changed since last frame.
    pub(crate) fn refresh_proxies_if_needed(&mut self) {
        if self.proxy_signature() != self.proxy_stamp {
            self.refresh_proxies();
        }
    }

    /// Playback > Use Optimised Media: the viewer reads the copies, or the files themselves.
    pub(crate) fn set_optimised(&mut self, on: bool) {
        if self.use_optimised == on {
            return;
        }
        self.use_optimised = on;
        self.extra.prefs.optimised_media = on;
        #[cfg(not(test))]
        self.extra.prefs.save();
        self.refresh_proxies();
        if on {
            if self.cache.uses_copies() {
                self.info("Playing from optimised media");
            } else {
                // It is on, but there is nothing to read as a copy yet: don't claim there is.
                self.info(
                    "Use Optimised Media is on, but no copy exists yet; make one in Media \
                     Management",
                );
            }
        } else {
            self.info("Playing from the original media");
        }
    }

    /// The video files of the media pool that have no copy yet, and are not being made.
    /// Files that are not on disk (offline) are skipped: there is nothing to read a copy from,
    /// and Optimised Media is only for files that play.
    pub(crate) fn optimise_wanted(&self) -> Vec<String> {
        self.pool_videos()
            .into_iter()
            .filter(|path| Path::new(path).is_file())
            .filter(|path| self.proxies.path_for(path).is_none())
            .filter(|path| !self.proxy_jobs.holds(path))
            .filter(|path| !self.proxy_jobs.is_unsuitable(path))
            .collect()
    }

    /// Makes the copies of the video files of the media pool, one after another.
    pub(crate) fn optimise_media(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            self.fail("optimised media needs the desktop app");
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !self.proxies.has_folder() {
                self.fail("there is no data folder to keep optimised media in");
                return;
            }
            let wanted = self.optimise_wanted();
            if wanted.is_empty() {
                self.info("Every video in the media pool has an optimised copy");
                return;
            }
            for path in &wanted {
                self.proxy_jobs.enqueue(path);
            }
            self.info(format!("Optimising {} file(s)...", wanted.len()));
            self.start_next_proxy();
        }
    }

    /// Starts the next copy waiting, if none is being made.
    pub(crate) fn start_next_proxy(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(source) = self.proxy_jobs.take() else {
                return;
            };
            let Some(dest) = self.proxies.place_for(&source) else {
                self.proxy_jobs.done(&source);
                return;
            };
            crate::proxy::start(
                source,
                dest,
                self.inbox.clone(),
                self.ctx.clone(),
                self.proxy_jobs.cancel_flag(),
            );
        }
    }

    /// The copy of `source` was made (where it is) or could not be. The next one starts.
    pub(crate) fn finish_proxy(&mut self, source: String, made: Result<String, String>) {
        self.proxy_jobs.done(&source);
        let mut stopped = false;
        match made {
            Ok(copy) => {
                self.proxies.record(&source, Path::new(&copy));
                self.proxies.save();
                self.info(format!("Optimised {}", name_of(&source)));
            }
            Err(e) if e == STOPPED => {
                stopped = true;
                self.info("Optimising stopped");
            }
            Err(e) if e == VARIABLE_RATE => {
                self.proxy_jobs.mark_unsuitable(&source);
                self.info(format!("{} is read as it is: {VARIABLE_RATE}", name_of(&source)));
            }
            Err(e) => self.fail(format!("could not optimise {}: {e}", name_of(&source))),
        }
        self.refresh_proxies();
        self.start_next_proxy();
        if !stopped && !self.proxy_jobs.busy() && !self.status.error && !self.proxy_jobs.is_unsuitable(&source) {
            self.info("The optimised copies are ready");
        }
    }

    /// How far the copy of `source` has got, when it is being made.
    pub(crate) fn proxy_progress(&mut self, source: &str, done: u64, total: u64) {
        self.proxy_jobs.progress(source, done, total);
        let percent = (done * 100).checked_div(total).unwrap_or(0);
        self.info(format!("Optimising {}: {percent}%", name_of(source)));
    }

    /// Stops making copies: the one being made is given up (its partial file removed) and the ones
    /// waiting are forgotten. The copies already made stay.
    pub(crate) fn cancel_proxies(&mut self) {
        if self.proxy_jobs.busy() {
            self.proxy_jobs.cancel_all();
            self.info("Stopping the optimised copies...");
        }
    }

    /// Deletes the copies of the video files of the media pool.
    pub(crate) fn forget_proxies(&mut self) {
        let wanted = self.pool_videos();
        let mut gone = 0;
        for (source, _) in self.proxies.copies() {
            if wanted.contains(&source) {
                self.proxies.forget(&source);
                gone += 1;
            }
        }
        self.proxies.save();
        self.refresh_proxies();
        if gone == 0 {
            self.info("The media pool's video files have no optimised copy");
        } else {
            self.info(format!("Deleted {gone} optimised copy(ies)"));
        }
    }

    /// Deletes every copy, for every project.
    pub(crate) fn delete_all_copies(&mut self) {
        let gone = self.proxies.copies().len();
        self.proxies.clear();
        self.proxies.save();
        self.refresh_proxies();
        self.info(format!("Deleted {gone} optimised copy(ies)"));
    }

    /// The cache an export reads: the originals, unless the Deliver page's "Use proxy media"
    /// checkbox (the "draft at proxy resolution" switch) is ticked. A copy is smaller than its
    /// file, so an export must not read one unless it was told to.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn export_cache(&self) -> picasso_render::MediaCache {
        if self.deliver_ui.proxy_media {
            self.cache.clone()
        } else {
            self.cache.originals()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-proxy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn source(dir: &Path, name: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, b"the original").unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_copy_is_found_by_the_file_it_was_made_from() {
        let dir = scratch("found");
        let store_dir = dir.join("store");
        let mut store = Store::load_from(Some(store_dir.clone()));
        let source = source(&dir, "A001.mov");
        let place = store.place_for(&source).unwrap();
        std::fs::create_dir_all(&store_dir).unwrap();
        std::fs::write(&place, b"the copy").unwrap();
        store.record(&source, &place);
        store.save();

        // A later start reads the index back.
        let again = Store::load_from(Some(store_dir));
        assert_eq!(again.path_for(&source), Some(place));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_index_cannot_name_a_file_outside_the_folder() {
        let dir = scratch("proxy_index_escape");
        let store_dir = dir.join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        let outside = dir.join("precious.txt");
        std::fs::write(&outside, b"keep me").unwrap();
        let source = source(&dir, "a.mov");
        let (size, modified) = stamp(Path::new(&source)).unwrap();
        std::fs::write(
            store_dir.join(INDEX),
            format!(
                "{source}\t../precious.txt\t{size}\t{modified}\n{source}2\t{}\t{size}\t{modified}\n",
                outside.display()
            ),
        )
        .unwrap();
        let mut store = Store::load_from(Some(store_dir));
        assert!(store.copies().is_empty(), "a path is not a file name: {:?}", store.copies());
        store.clear();
        assert!(outside.exists(), "nothing outside the folder was deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_paths_with_one_hash_do_not_share_a_name() {
        let dir = scratch("proxy_same_name");
        let mut store = Store::load_from(Some(dir.join("store")));
        let one = source(&dir, "x.mov");
        let first = store.place_for(&one).unwrap();
        // Another file whose entry already holds that name.
        store.entries.insert(
            "another".into(),
            Entry {
                file: first.file_name().unwrap().to_string_lossy().into_owned(),
                size: 1,
                modified: 1,
            },
        );
        let second = store.place_for(&one).unwrap();
        assert_ne!(first, second, "the name is taken, so the next is used");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_name_the_index_cannot_hold_keeps_no_copy() {
        let store = Store::load_from(Some(std::env::temp_dir().join("picasso_proxy_odd")));
        assert!(store.place_for("a\tb.mov").is_none());
        assert!(store.place_for("a\nb.mov").is_none());
        assert!(store.place_for("ab.mov").is_some());
    }

    #[test]
    fn a_copy_never_finished_is_swept_when_it_is_old_enough() {
        let dir = scratch("proxy_sweep");
        let store_dir = dir.join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        let source = source(&dir, "a.mov");
        let mut store = Store::load_from(Some(store_dir.clone()));
        let place = store.place_for(&source).unwrap();
        std::fs::write(&place, b"the copy").unwrap();
        store.record(&source, &place);
        let partial = store_dir.join("half-0000000000000001.mp4");
        std::fs::write(&partial, b"half").unwrap();
        // Just written: another program may be making it, so it stays.
        assert_eq!(store.sweep(), 0);
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&partial)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(store.sweep(), 1);
        assert!(!partial.exists() && place.exists(), "the unfinished one went, the copy stayed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_files_of_one_name_in_different_folders_do_not_share_a_copy() {
        let dir = scratch("names");
        let store = Store::load_from(Some(dir.join("store")));
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        let one = source(&dir.join("a"), "clip.mov");
        let two = source(&dir.join("b"), "clip.mov");
        assert_ne!(store.place_for(&one), store.place_for(&two));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_of_a_file_that_changed_since_is_not_used() {
        let dir = scratch("stale");
        let store_dir = dir.join("store");
        let mut store = Store::load_from(Some(store_dir.clone()));
        let source = source(&dir, "A001.mov");
        let place = store.place_for(&source).unwrap();
        std::fs::create_dir_all(&store_dir).unwrap();
        std::fs::write(&place, b"the copy").unwrap();
        store.record(&source, &place);
        assert!(store.path_for(&source).is_some());

        // The file is written again: the copy no longer stands for it.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&source, b"the original, but longer").unwrap();
        assert_eq!(store.path_for(&source), None);
        assert_eq!(store.prune(), 1, "and pruning drops it");
        assert_eq!(store.path_for(&source), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_whose_file_is_gone_is_not_used() {
        let dir = scratch("gone");
        let store_dir = dir.join("store");
        let mut store = Store::load_from(Some(store_dir.clone()));
        let source = source(&dir, "A001.mov");
        let place = store.place_for(&source).unwrap();
        std::fs::create_dir_all(&store_dir).unwrap();
        std::fs::write(&place, b"the copy").unwrap();
        store.record(&source, &place);
        std::fs::remove_file(&place).unwrap();
        assert_eq!(store.path_for(&source), None, "a copy that is not there is not used");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forgetting_and_clearing_delete_the_files_too() {
        let dir = scratch("clear");
        let store_dir = dir.join("store");
        let mut store = Store::load_from(Some(store_dir.clone()));
        std::fs::create_dir_all(&store_dir).unwrap();
        let one = source(&dir, "one.mov");
        let two = source(&dir, "two.mov");
        let a = store.place_for(&one).unwrap();
        let b = store.place_for(&two).unwrap();
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        store.record(&one, &a);
        store.record(&two, &b);

        store.forget(&one);
        assert!(!a.is_file(), "forgetting removes the copy");
        assert!(b.is_file(), "and only that one");

        store.clear();
        assert!(!b.is_file(), "clearing removes the rest");
        assert!(store.copies().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_is_at_most_1280_on_its_longest_side_and_keeps_its_shape() {
        assert_eq!(copy_size((3840, 2160)), (1280, 720));
        assert_eq!(copy_size((1920, 1080)), (1280, 720));
        assert_eq!(copy_size((1080, 1920)), (720, 1280));
        assert_eq!(copy_size((1280, 720)), (1280, 720), "no bigger than the file");
        assert_eq!(copy_size((640, 360)), (640, 360));
        // An odd size stays even, which the encoders want.
        assert_eq!(copy_size((1919, 1079)), (1280, 720));
    }

    #[test]
    fn a_file_already_small_enough_is_kept_at_its_own_size() {
        assert_eq!(copy_size((720, 480)), (720, 480));
    }

    /// Two seconds of `size` at 25 fps; `None` when the `ffmpeg` program is not installed.
    fn video(dir: &Path, name: &str, size: &str) -> Option<PathBuf> {
        let path = dir.join(name);
        std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc=duration=2:size={size}:rate=25"))
            .args(["-pix_fmt", "yuv420p", "-g", "50"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success())
            .then_some(path)
    }

    #[test]
    fn a_copy_is_the_same_length_and_frame_rate_at_a_smaller_size() {
        let dir = scratch("make");
        let Some(source) = video(&dir, "A001.mp4", "1920x1080") else {
            return; // no ffmpeg here
        };
        let dest = dir.join("copy.mp4");
        let mut frames = (0, 0);
        make(&source, &dest, &mut |done, total| {
            frames = (done, total);
            true
        })
        .unwrap();

        assert!(dest.is_file(), "the copy was written");
        let info = picasso_decode::probe(&dest).unwrap();
        assert_eq!((info.width, info.height), (1280, 720), "1280 on the long side");
        // The frame rate is the file's own. The container's average reads a hair over, because
        // the last picture is written without a length (an MP4 the export writes is one picture
        // short); the pictures themselves are where the original's are, which is what matters.
        let rate = f64::from(info.fps.0) / f64::from(info.fps.1.max(1));
        assert!((rate - 25.0).abs() < 0.6, "25 a second, not {rate}");
        let seconds = info.duration_us.unwrap() as f64 / 1e6;
        assert!((seconds - 2.0).abs() < 0.15, "two seconds, not {seconds}");
        assert_eq!(frames.1, 50, "50 pictures to write");
        assert_eq!(frames.0, 50, "and all of them written");

        // The same time is the same picture: one second in, the copy's picture starts at one
        // second, so a project that asks for a time gets the picture it asks for.
        let mut decoder = picasso_decode::VideoDecoder::open(&dest).unwrap();
        use picasso_decode::FrameProvider;
        let frame = decoder.frame_at(1_000_000).unwrap();
        assert!(
            (frame.pts_us - 1_000_000).abs() < 40_000,
            "one second in is one second in, not {}",
            frame.pts_us
        );
        assert_eq!((frame.width, frame.height), (1280, 720));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An app with one video on its timeline, the file being `path`.
    fn app_with_video(path: &str) -> StudioApp {
        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        app.load_demo();
        // The demo's own media are not files; the video is what the window works on.
        let media = app
            .run(picasso_core::Command::AddMedia {
                name: "A001".into(),
                source: picasso_core::MediaSource::Video {
                    path: path.to_string(),
                },
                length: Some(50),
                default_duration: 50,
            })
            .expect("the file was added");
        let picasso_core::Outcome::Media(media) = media else {
            panic!("no media");
        };
        app.insert_media(media);
        app
    }

    /// A store with a copy of `source` already made, and the app told about it.
    fn with_copy(app: &mut StudioApp, dir: &Path, source: &str) -> PathBuf {
        let mut store = Store::load_from(Some(dir.join("store")));
        std::fs::create_dir_all(dir.join("store")).unwrap();
        let place = store.place_for(source).unwrap();
        std::fs::write(&place, b"the copy").unwrap();
        store.record(source, &place);
        app.proxies = store;
        app.refresh_proxies();
        place
    }

    #[test]
    fn the_viewer_reads_the_copy_only_when_there_is_one_and_it_is_wanted() {
        let dir = scratch("wired");
        let source = source(&dir, "A001.mov");
        let mut app = app_with_video(&source);
        assert_eq!(app.cache.copy_for(&source), None, "no copy yet");

        let place = with_copy(&mut app, &dir, &source);
        assert_eq!(
            app.cache.copy_for(&source),
            Some(place.to_string_lossy().as_ref()),
            "the viewer reads the copy"
        );

        // Playback > Use Optimised Media off: the file itself is read again.
        app.set_optimised(false);
        assert_eq!(app.cache.copy_for(&source), None);
        assert!(!app.cache.uses_copies());
        app.set_optimised(true);
        assert!(app.cache.uses_copies());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_of_a_file_that_changed_is_not_given_to_the_viewer() {
        let dir = scratch("changed");
        let source = source(&dir, "A001.mov");
        let mut app = app_with_video(&source);
        with_copy(&mut app, &dir, &source);
        // The file is written again: the copy no longer stands for it.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&source, b"the original, but longer").unwrap();
        app.refresh_proxies();
        assert_eq!(app.cache.copy_for(&source), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_export_reads_the_files_unless_it_is_told_to_use_the_copies() {
        let dir = scratch("export");
        let source = source(&dir, "A001.mov");
        let mut app = app_with_video(&source);
        with_copy(&mut app, &dir, &source);
        assert!(app.cache.uses_copies(), "the viewer reads the copy");
        assert!(
            !app.export_cache().uses_copies(),
            "an export reads the file, whole"
        );
        app.deliver_ui.proxy_media = true;
        assert!(
            app.export_cache().uses_copies(),
            "ticked on the Deliver page, it reads the copy"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_files_without_a_copy_are_queued() {
        let dir = scratch("wanted");
        let one = source(&dir, "A001.mov");
        let two = source(&dir, "A002.mov");
        let mut app = app_with_video(&one);
        let media = app
            .run(picasso_core::Command::AddMedia {
                name: "A002".into(),
                source: picasso_core::MediaSource::Video {
                    path: two.clone(),
                },
                length: Some(50),
                default_duration: 50,
            })
            .and_then(|o| match o {
                picasso_core::Outcome::Media(id) => Some(id),
                _ => None,
            })
            .unwrap();
        // Past the first clip: one clip per place on the track.
        app.playhead = 50;
        app.insert_media(media);
        with_copy(&mut app, &dir, &one);
        assert_eq!(app.optimise_wanted(), vec![two.clone()]);
        // Once it is in line it is not asked for again.
        app.proxy_jobs.enqueue(&two);
        assert!(app.optimise_wanted().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_video_in_the_pool_on_no_timeline_is_optimised_and_deleted_like_the_others() {
        let dir = scratch("pool");
        let used = source(&dir, "A001.mov");
        let unused = source(&dir, "A002.mov");
        let mut app = app_with_video(&used);
        // Imported to the pool, never put on a timeline; and the same file imported again.
        for name in ["A002", "A002 again"] {
            app.run(picasso_core::Command::AddMedia {
                name: name.into(),
                source: picasso_core::MediaSource::Video {
                    path: unused.clone(),
                },
                length: Some(50),
                default_duration: 50,
            })
            .expect("the file was added");
        }
        assert_eq!(app.pool_videos(), vec![used.clone(), unused.clone()], "each file once");
        assert_eq!(app.optimise_wanted(), vec![used.clone(), unused.clone()]);

        // A copy of the unused one is read by the viewer, listed, and deleted by Delete Copies.
        let place = with_copy(&mut app, &dir, &unused);
        assert_eq!(app.optimise_wanted(), vec![used.clone()], "only the other one is wanted");
        assert!(app.cache.copy_for(&unused).is_some());
        app.forget_proxies();
        assert!(!place.is_file(), "the copy of a video on no timeline goes too");
        assert_eq!(app.cache.copy_for(&unused), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_the_copies_leaves_the_other_projects_ones() {
        let dir = scratch("delete");
        let mine = source(&dir, "A001.mov");
        let theirs = source(&dir, "B002.mov");
        let mut app = app_with_video(&mine);
        let store_dir = dir.join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        let mut store = Store::load_from(Some(store_dir.clone()));
        let a = store.place_for(&mine).unwrap();
        let b = store.place_for(&theirs).unwrap();
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        store.record(&mine, &a);
        store.record(&theirs, &b);
        app.proxies = store;
        app.refresh_proxies();

        app.forget_proxies();
        assert!(!a.is_file(), "this project's copy goes");
        assert!(b.is_file(), "another project's copy stays");
        assert_eq!(app.cache.copy_for(&mine), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelling_forgets_the_waiting_and_stops_the_one_being_made() {
        let mut jobs = Jobs::default();
        jobs.enqueue("a.mov");
        jobs.enqueue("b.mov");
        jobs.enqueue("c.mov");
        assert_eq!(jobs.take().as_deref(), Some("a.mov"));
        let flag = jobs.cancel_flag();
        assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
        jobs.cancel_all();
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed), "the thread is told to stop");
        assert!(!jobs.holds("b.mov") && !jobs.holds("c.mov"), "the ones waiting are forgotten");
        assert!(jobs.holds("a.mov"), "the one being made stays until it reports back");
        jobs.done("a.mov");
        assert!(!jobs.busy());
        // The next job starts with the flag down again.
        jobs.enqueue("d.mov");
        assert_eq!(jobs.take().as_deref(), Some("d.mov"));
        assert!(!jobs.cancel_flag().load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn a_file_found_unable_to_have_a_copy_is_not_asked_for_again() {
        let mut jobs = Jobs::default();
        assert!(!jobs.is_unsuitable("phone.mp4"));
        jobs.mark_unsuitable("phone.mp4");
        assert!(jobs.is_unsuitable("phone.mp4") && !jobs.is_unsuitable("other.mp4"));
    }

    #[test]
    fn a_copy_being_made_can_be_stopped_and_leaves_nothing_but_the_error() {
        let dir = scratch("proxy_stop");
        let Some(file) = video(&dir, "long.mp4", "640x360") else {
            return; // no ffmpeg here
        };
        let dest = dir.join("copy.mp4");
        // Stops at the third picture.
        let result = make(&file, &dest, &mut |done, _| done < 3);
        assert!(result.is_err(), "a copy that was stopped is an error, not a copy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_whose_frame_rate_varies_is_not_given_a_copy() {
        let dir = scratch("proxy_vfr");
        let path = dir.join("vfr.mp4");
        // Pictures dropped here and there: the rate the file runs at and its average differ.
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=duration=4:size=320x180:rate=60"])
            .args(["-vf", "select='lt(mod(n,10),3)+gt(n,150)'", "-vsync", "vfr"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return; // no ffmpeg here
        }
        let info = picasso_decode::probe(&path).unwrap();
        assert!(info.variable_frame_rate, "the rate varies: {info:?}");
        let result = make(&path, &dir.join("copy.mp4"), &mut |_, _| true);
        assert_eq!(result, Err(VARIABLE_RATE.to_string()));
        // A fixed rate is still given a copy.
        if let Some(fixed) = video(&dir, "fixed.mp4", "320x180") {
            assert!(!picasso_decode::probe(&fixed).unwrap().variable_frame_rate);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_are_made_one_at_a_time_and_say_how_far_they_are() {
        let mut jobs = Jobs::default();
        assert!(jobs.enqueue("a.mov"));
        assert!(jobs.enqueue("b.mov"));
        assert!(!jobs.enqueue("a.mov"), "it is already in line");
        assert!(jobs.holds("a.mov") && jobs.holds("b.mov"));

        assert_eq!(jobs.take(), Some("a.mov".to_string()));
        assert_eq!(jobs.take(), None, "b waits while a is being made");
        jobs.progress("a.mov", 25, 50);
        assert_eq!(jobs.fraction("a.mov"), Some(0.5));
        assert_eq!(jobs.fraction("b.mov"), None, "b has not started");

        jobs.done("a.mov");
        assert!(!jobs.holds("a.mov"));
        assert_eq!(jobs.take(), Some("b.mov".to_string()));
        jobs.done("b.mov");
        assert!(!jobs.busy(), "nothing is left");
    }

    #[test]
    fn a_copy_that_was_made_is_kept_for_the_next_start() {
        let dir = scratch("record");
        let source = source(&dir, "A001.mov");
        let mut app = app_with_video(&source);
        let store_dir = dir.join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        app.proxies = Store::load_from(Some(store_dir.clone()));
        let place = app.proxies.place_for(&source).unwrap();
        std::fs::write(&place, b"the copy").unwrap();

        app.finish_proxy(source.clone(), Ok(place.to_string_lossy().into_owned()));
        assert_eq!(app.cache.copy_for(&source), Some(place.to_string_lossy().as_ref()));
        // The next start finds it, without being told again.
        let again = Store::load_from(Some(store_dir));
        assert_eq!(again.path_for(&source), Some(place));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End to end, with ffmpeg: queue a file, let the worker render a real copy, take it in
    /// through the inbox, and check the viewer now reads the copy. Ignored by default: it takes
    /// a few seconds and needs the `ffmpeg` program.
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    fn optimise_media_makes_the_copy_the_viewer_reads() {
        // A real 1080p two-second file; bail without one.
        let dir = scratch("e2e");
        let source = video(&dir, "A001.mp4", "1920x1080")
            .map(|p| p.to_string_lossy().into_owned())
            .or(None);
        let Some(source) = source else { return };
        let mut app = app_with_video(&source);
        let store_dir = dir.join("store");
        std::fs::create_dir_all(&store_dir).unwrap();
        app.proxies = Store::load_from(Some(store_dir.clone()));
        assert!(app.optimise_wanted().contains(&source), "it has no copy yet");
        app.optimise_media();

        // The worker runs; drain the inbox until the copy is reported made.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(100));
            app.process_inbox();
            if app.cache.copy_for(&source).is_some() {
                break;
            }
        }
        let copy = app.cache.copy_for(&source);
        assert!(copy.is_some(), "the viewer now reads a copy: {}", app.status.text);
        let copy = std::path::PathBuf::from(copy.unwrap());
        assert!(copy.is_file());
        // ...and so does the next start.
        let again = Store::load_from(Some(store_dir.clone()));
        assert_eq!(again.path_for(&source), Some(copy.clone()));
        let info = picasso_decode::probe(&copy).unwrap();
        assert_eq!((info.width, info.height), (1280, 720));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
