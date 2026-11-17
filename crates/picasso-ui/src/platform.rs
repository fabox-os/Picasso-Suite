//! What differs between the desktop and the browser: file dialogs and background work.
//!
//! Dialogs run as futures. On the desktop a thread drives each one; in the browser the page's
//! event loop does. The result comes back through an [`Inbox`] that the UI empties every frame.

use std::future::Future;
use std::sync::{Arc, Mutex};

/// A file the user gave Picasso Suite, already read.
pub(crate) struct ImportedFile {
    pub name: String,
    /// Where it was on disk; the browser does not say.
    pub path: Option<String>,
    pub bytes: Vec<u8>,
}

/// Something that finished in the background.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // choosing a folder is desktop only
pub(crate) enum Incoming {
    Media(Vec<ImportedFile>),
    /// A project file: a project database or JSON text.
    Project {
        name: String,
        path: Option<String>,
        bytes: Vec<u8>,
    },
    /// The place to save to was not chosen.
    SaveCancelled,
    /// A file was written; `project` says whether it was the project.
    Saved {
        name: String,
        path: Option<String>,
        project: bool,
    },
    /// A look-up table (`.cube` text) to put on the clip being graded.
    Lut {
        name: String,
        bytes: Vec<u8>,
    },
    /// A folder was chosen (desktop only).
    Folder(String),
    /// A file to render into was chosen (desktop only).
    ExportFile(String),
    /// A folder to import as a bin was chosen (desktop only).
    BinFolder(String),
    /// A folder to look in for media whose files moved was chosen (desktop only).
    RelinkFolder(String),
    /// The media found in that folder, read in the background (desktop only).
    Relinked {
        found: Vec<crate::relink::FoundFile>,
        /// Names no file under the folder has.
        absent: Vec<String>,
        /// Names more than one file has.
        ambiguous: Vec<String>,
        /// What each found file says about itself.
        facts: Vec<(picasso_core::MediaId, Option<crate::projectfile::MediaMetadata>)>,
        folder: String,
    },
    /// A folder to archive the project into was chosen (desktop only).
    ArchiveFolder(String),
    /// How far the archive has got: files copied of the total.
    ArchiveProgress {
        done: usize,
        total: usize,
    },
    /// The archive was written: what was copied, what was not there, and what failed.
    Archived {
        folder: String,
        copied: usize,
        missing: Vec<String>,
        failed: Vec<String>,
    },
    /// The bounced mix was written to this file (desktop only).
    Bounced(String),
    /// A clip was rendered to this file, to take its place (desktop only).
    Rendered {
        clip: picasso_core::ClipId,
        path: String,
    },
    /// A video or sound file read in the background: what its file says, ready to import
    /// (desktop only).
    Probed {
        file: ImportedFile,
        sound_only: bool,
        probed: Result<picasso_media::MediaMetadata, String>,
    },
    /// A DaVinci Resolve file, unpacked and read in the background (desktop only).
    Resolved {
        name: String,
        read: Result<picasso_drp::DrpProject, String>,
        /// The app's epoch when it was asked for: another project opened since wins.
        epoch: u64,
    },
    /// An OpenTimelineIO timeline, with the media files it uses read in the background.
    OtioRead {
        name: String,
        timeline: picasso_core::otio::OtioTimeline,
        probed: Vec<crate::interchange::Probed>,
        /// The app's epoch when it was asked for: another project opened since wins.
        epoch: u64,
    },
    /// Another editor's timeline file, read in the background because it is large.
    TimelineRead {
        name: String,
        read: Result<picasso_core::otio::OtioTimeline, String>,
        /// The app's epoch when it was asked for: another project opened since wins.
        epoch: u64,
    },
    /// The codec of a video, read from its file in the background (desktop only).
    CodecLabel {
        media: picasso_core::MediaId,
        label: String,
    },
    /// How far the optimised copy of a file has got: pictures written of the total.
    ProxyProgress {
        source: String,
        done: u64,
        total: u64,
    },
    /// The optimised copy of a file was made (where it is), or could not be.
    ProxyDone {
        source: String,
        made: Result<String, String>,
    },
    Error(String),
}

/// A queue the background tasks put their results in.
#[derive(Clone, Default)]
pub(crate) struct Inbox(Arc<Mutex<Vec<Incoming>>>);

impl Inbox {
    pub fn push(&self, ctx: &egui::Context, item: Incoming) {
        if let Ok(mut queue) = self.0.lock() {
            queue.push(item);
        }
        ctx.request_repaint();
    }

    pub fn take(&self) -> Vec<Incoming> {
        self.0
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(future: impl Future<Output = ()> + Send + 'static) {
    std::thread::spawn(move || pollster::block_on(future));
}

#[cfg(target_arch = "wasm32")]
fn spawn(future: impl Future<Output = ()> + 'static) {
    wasm_bindgen_futures::spawn_local(future);
}

#[cfg(not(target_arch = "wasm32"))]
fn path_of(handle: &rfd::FileHandle) -> Option<String> {
    Some(handle.path().to_string_lossy().into_owned())
}

#[cfg(target_arch = "wasm32")]
fn path_of(_: &rfd::FileHandle) -> Option<String> {
    None
}

/// File extensions of a project.
pub(crate) const PROJECT_EXTENSIONS: [&str; 2] = ["picasso", "json"];

/// File extensions of video the desktop app can read through FFmpeg.
pub(crate) const VIDEO_EXTENSIONS: [&str; 21] = [
    "mp4", "mov", "mkv", "webm", "avi", "mxf", "m4v", "ts", "mts", "m2ts", "wmv", "flv", "mpg",
    "mpeg", "ogv", "3gp", "3g2", "f4v", "vob", "dv", "asf",
];

pub(crate) fn is_video_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    VIDEO_EXTENSIONS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// File extensions of sound the desktop app can read through FFmpeg.
pub(crate) const AUDIO_EXTENSIONS: [&str; 10] = [
    "wav", "mp3", "flac", "aac", "m4a", "ogg", "opus", "aif", "aiff", "wma",
];

pub(crate) fn is_audio_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    AUDIO_EXTENSIONS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// Media that FFmpeg reads from its path (so the bytes are not read in advance).
pub(crate) fn is_path_media(name: &str) -> bool {
    is_video_name(name) || is_audio_name(name)
}

/// DaVinci Resolve's files that can be opened: projects, timelines, bins and XML exports.
pub(crate) const RESOLVE_EXTENSIONS: [&str; 4] = ["drp", "drt", "drb", "xml"];

pub(crate) fn is_resolve_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    RESOLVE_EXTENSIONS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

pub(crate) fn is_project_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PROJECT_EXTENSIONS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// Lets the user pick images to import.
pub(crate) fn pick_media(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        let Some(handles) = rfd::AsyncFileDialog::new()
            .set_title("Import media")
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .add_filter("Video", &VIDEO_EXTENSIONS)
            .add_filter("Audio", &AUDIO_EXTENSIONS)
            .pick_files()
            .await
        else {
            return;
        };
        let mut files = Vec::new();
        for handle in handles {
            // A video is read by FFmpeg from its path; reading it all into memory would be slow.
            let bytes = if cfg!(not(target_arch = "wasm32")) && is_path_media(&handle.file_name()) {
                Vec::new()
            } else {
                handle.read().await
            };
            files.push(ImportedFile {
                name: handle.file_name(),
                path: path_of(&handle),
                bytes,
            });
        }
        inbox.push(&ctx, Incoming::Media(files));
    });
}

/// Lets the user pick a look-up table (`.cube`).
pub(crate) fn pick_lut(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Apply LUT")
            .add_filter("Cube LUT", &["cube"])
            .pick_file()
            .await
        else {
            return;
        };
        let bytes = handle.read().await;
        inbox.push(
            &ctx,
            Incoming::Lut {
                name: handle.file_name(),
                bytes,
            },
        );
    });
}

/// The largest picture the browser keeps: the roadmap's 720p proxy, so that a 4K still does
/// not fill the page's memory.
pub(crate) const BROWSER_PICTURE: (u32, u32) = (1280, 720);

/// A picture as this build keeps it: whole on the desktop, at most [`BROWSER_PICTURE`] in the
/// browser.
pub(crate) fn fit_picture(image: picasso_render::Decoded) -> picasso_render::Decoded {
    if cfg!(target_arch = "wasm32") {
        image.fit_within(BROWSER_PICTURE.0, BROWSER_PICTURE.1)
    } else {
        image
    }
}

/// Timelines and subtitles from other programs, opened as the project's files are.
pub(crate) fn is_timeline_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".otio", ".aaf", ".edl", ".srt", ".vtt"]
        .iter()
        .any(|e| lower.ends_with(e))
}

pub(crate) fn is_lut_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".cube")
}

/// Effect bundles: `.picef` (the roadmap's name).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use picasso_drp::bundle::is_bundle_name;

#[cfg(target_arch = "wasm32")]
pub(crate) const BUNDLE_EXTENSIONS: [&str; 1] = ["picef"];

/// Whether a file name is an effect bundle (the browser cannot install one, but says so).
#[cfg(target_arch = "wasm32")]
pub(crate) fn is_bundle_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    BUNDLE_EXTENSIONS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// Whether a file on disk is bigger than an effect bundle may be. The browser has no path to ask
/// about, so nothing is refused here.
fn bundle_is_too_big(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() > picasso_drp::bundle::MAX_BUNDLE_BYTES)
        .unwrap_or(false)
}

/// Lets the user pick an effect bundle (`.picef`) to install.
pub(crate) fn pick_bundle(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Install Effect Bundle")
            .add_filter("Effect bundle", &["picef"])
            .pick_file()
            .await
        else {
            return;
        };
        // Refuse an oversized bundle before reading it into memory (desktop only: the browser has
        // no path to ask about).
        if path_of(&handle).is_some_and(|p| bundle_is_too_big(std::path::Path::new(&p))) {
            inbox.push(
                &ctx,
                Incoming::Error(format!(
                    "{}: the bundle is too big to read",
                    handle.file_name()
                )),
            );
            return;
        }
        let bytes = handle.read().await;
        inbox.push(
            &ctx,
            Incoming::Media(vec![ImportedFile {
                name: handle.file_name(),
                path: path_of(&handle),
                bytes,
            }]),
        );
    });
}

/// Lets the user pick a project to open.
pub(crate) fn pick_project(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Open project")
            .add_filter("Picasso Suite project", &PROJECT_EXTENSIONS)
            .add_filter("DaVinci Resolve project", &RESOLVE_EXTENSIONS)
            .add_filter("Edit decision list", &["edl"])
            .add_filter("OpenTimelineIO", &["otio"])
            .add_filter("Final Cut Pro 7 / Premiere XML", &["xml"])
            .add_filter("AAF", &["aaf"])
            .add_filter("Subtitles", &["srt", "vtt"])
            .pick_file()
            .await
        else {
            return;
        };
        let bytes = handle.read().await;
        inbox.push(
            &ctx,
            Incoming::Project {
                name: handle.file_name(),
                path: path_of(&handle),
                bytes,
            },
        );
    });
}

/// Writes `bytes` to `path` on a background thread, so a slow disk or a virus scanner looking at
/// the new file never stalls the window. The result arrives through the inbox.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_project(inbox: Inbox, ctx: egui::Context, path: String, bytes: Vec<u8>) {
    std::thread::spawn(move || {
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        match std::fs::write(&path, &bytes) {
            Ok(()) => inbox.push(
                &ctx,
                Incoming::Saved {
                    name,
                    path: Some(path),
                    project: true,
                },
            ),
            Err(e) => inbox.push(&ctx, Incoming::Error(format!("could not save {path}: {e}"))),
        }
    });
}

/// Asks where to save `bytes`, then writes them. In the browser this is a download.
pub(crate) fn save_as(
    inbox: Inbox,
    ctx: egui::Context,
    suggested: String,
    filter: (&'static str, Vec<&'static str>),
    bytes: Vec<u8>,
    project: bool,
) {
    spawn(async move {
        let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Save")
            .set_file_name(suggested)
            .add_filter(filter.0, &filter.1)
            .save_file()
            .await
        else {
            inbox.push(&ctx, Incoming::SaveCancelled);
            return;
        };
        match handle.write(&bytes).await {
            Ok(()) => inbox.push(
                &ctx,
                Incoming::Saved {
                    name: handle.file_name(),
                    path: path_of(&handle),
                    project,
                },
            ),
            Err(e) => inbox.push(&ctx, Incoming::Error(format!("could not save: {e}"))),
        }
    });
}

/// Lets the user pick a folder to export into.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pick_folder(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        if let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Export into folder")
            .pick_folder()
            .await
        {
            inbox.push(
                &ctx,
                Incoming::Folder(handle.path().to_string_lossy().into_owned()),
            );
        }
    });
}

/// Lets the user pick a folder whose media is imported into a new bin.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pick_bin_folder(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        if let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Import folder as bin")
            .pick_folder()
            .await
        {
            inbox.push(
                &ctx,
                Incoming::BinFolder(handle.path().to_string_lossy().into_owned()),
            );
        }
    });
}

/// Lets the user pick a folder to look in for media whose files moved.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pick_relink_folder(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        if let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Find the media in")
            .pick_folder()
            .await
        {
            inbox.push(
                &ctx,
                Incoming::RelinkFolder(handle.path().to_string_lossy().into_owned()),
            );
        }
    });
}

/// Lets the user pick the folder the archive of the project goes into.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pick_archive_folder(inbox: Inbox, ctx: egui::Context) {
    spawn(async move {
        if let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Archive project in")
            .pick_folder()
            .await
        {
            inbox.push(
                &ctx,
                Incoming::ArchiveFolder(handle.path().to_string_lossy().into_owned()),
            );
        }
    });
}

/// Lets the user choose the file a render goes into.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pick_export_file(
    inbox: Inbox,
    ctx: egui::Context,
    suggested: String,
    extension: &'static str,
) {
    spawn(async move {
        if let Some(handle) = rfd::AsyncFileDialog::new()
            .set_title("Render to")
            .set_file_name(suggested)
            .add_filter(extension.to_uppercase(), &[extension])
            .save_file()
            .await
        {
            inbox.push(
                &ctx,
                Incoming::ExportFile(handle.path().to_string_lossy().into_owned()),
            );
        }
    });
}

/// Whether images can be re-read from the paths a project remembers.
pub(crate) const CAN_READ_PATHS: bool = cfg!(not(target_arch = "wasm32"));

/// Reads a file the project remembers by path. Always `None` in the browser.
pub(crate) fn read_path(path: &str) -> Option<Vec<u8>> {
    if CAN_READ_PATHS && picasso_decode::refuse_path(path).is_none() {
        std::fs::read(path).ok()
    } else {
        None
    }
}

/// Reads the files the user dropped on the window and queues them as imports. On the desktop
/// they are read at once; in the browser the page reads them and the result arrives later.
pub(crate) fn queue_dropped(files: Vec<egui::DroppedFileHandle>, inbox: Inbox, ctx: egui::Context) {
    fn name_of(path: &std::path::Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut read = Vec::new();
        for file in &files {
            let name = name_of(file.path());
            // Refuse an oversized bundle before reading it into memory.
            if is_bundle_name(&name) && bundle_is_too_big(file.path()) {
                inbox.push(
                    &ctx,
                    Incoming::Error(format!("{name}: the bundle is too big to read")),
                );
                continue;
            }
            let bytes = if is_path_media(&name) {
                Ok(Vec::new())
            } else {
                file.bytes()
            };
            match bytes {
                Ok(bytes) => read.push(ImportedFile {
                    name,
                    path: Some(file.path().to_string_lossy().into_owned()),
                    bytes,
                }),
                Err(e) => inbox.push(&ctx, Incoming::Error(format!("{name}: {e}"))),
            }
        }
        if !read.is_empty() {
            inbox.push(&ctx, Incoming::Media(read));
        }
    }
    #[cfg(target_arch = "wasm32")]
    spawn(async move {
        let mut read = Vec::new();
        for file in &files {
            match file.bytes_async().await {
                Ok(bytes) => read.push(ImportedFile {
                    name: name_of(file.path()),
                    path: None,
                    bytes,
                }),
                Err(e) => inbox.push(
                    &ctx,
                    Incoming::Error(format!("{}: {e}", name_of(file.path()))),
                ),
            }
        }
        if !read.is_empty() {
            inbox.push(&ctx, Incoming::Media(read));
        }
    });
}
