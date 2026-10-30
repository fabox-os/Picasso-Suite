//! File > Media Management: what is done to a project's files as a whole.
//!
//! - Relink Missing Media: find the files that moved; the search itself is in
//!   [`crate::relink`].
//! - Archive Project: copy the files the timelines use into a folder, and write a copy of the
//!   project there that points at them, so that folder can be moved, handed over or backed up
//!   as one thing. The project being edited is not touched: the archive is a copy.
//! - Optimised Media: make small copies of the video files that decode quickly, for smooth
//!   editing; the copies themselves are in [`crate::proxy`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::Context;
use picasso_core::{Command, MediaId, MediaSource};

use crate::app::StudioApp;
use crate::platform::{self, Incoming};

/// A project about to be archived: where the copy goes and what to copy.
pub(crate) struct Archive {
    /// The folder the project copy goes into (`<chosen>/<project>`).
    pub root: PathBuf,
    /// The project file inside it.
    pub project_file: PathBuf,
    /// The folder the media goes into.
    pub media_dir: PathBuf,
    /// The bytes of the project copy, pointing at the copied files.
    pub project: Vec<u8>,
    /// Each file to copy, from where it is to where the copy goes.
    pub copies: Vec<(PathBuf, PathBuf)>,
    /// Media whose file is not here: not copied, and the copy still points at the old path.
    pub missing: Vec<String>,
}

impl Archive {
    /// Copies the files and writes the project, saying how far it is. Returns how many files
    /// were copied and what could not be. Run on a background thread.
    fn write(&self, inbox: &platform::Inbox, ctx: &Context) -> (usize, Vec<String>) {
        let mut copied = 0;
        let mut failed = Vec::new();
        if let Err(e) = std::fs::create_dir_all(&self.media_dir) {
            return (0, vec![format!("{}: {e}", self.media_dir.display())]);
        }
        let total = self.copies.len();
        for (i, (src, dest)) in self.copies.iter().enumerate() {
            match std::fs::copy(src, dest) {
                Ok(_) => copied += 1,
                Err(e) => failed.push(format!("{}: {e}", src.display())),
            }
            // Every few files, so a large archive says it is still working.
            if i % 8 == 0 || i + 1 == total {
                inbox.push(ctx, Incoming::ArchiveProgress { done: i + 1, total });
            }
        }
        if let Err(e) = std::fs::write(&self.project_file, &self.project) {
            failed.push(format!("{}: {e}", self.project_file.display()));
        }
        (copied, failed)
    }
}

impl StudioApp {
    /// File > Media Management opens the window.
    pub(crate) fn show_media_management(&mut self) {
        self.extra.media_management = true;
        // Copies whose file changed or went are of no use to anyone: they go when the window
        // that manages them opens.
        if self.proxies.prune() > 0 {
            self.proxies.save();
            self.refresh_proxies();
        }
    }

    /// The Media Management window: relink, archive, and optimised media.
    pub(crate) fn media_management_window(&mut self, ctx: &Context) {
        if !self.extra.media_management {
            return;
        }
        let mut open = true;
        egui::Window::new("Media Management")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(470.0)
            .show(ctx, |ui| {
                let missing = self.missing_media();
                ui.heading("Relink Missing Media");
                if missing.is_empty() {
                    ui.label("Every media file is where the project says it is.");
                } else {
                    ui.label(format!(
                        "{} media file(s) are not where the project says they are.",
                        missing.len()
                    ));
                }
                if ui
                    .add_enabled(
                        !missing.is_empty(),
                        egui::Button::new("Relink Missing Media..."),
                    )
                    .clicked()
                {
                    self.relink_missing(ui.ctx());
                }
                ui.separator();
                ui.heading("Archive Project");
                ui.label(
                    "Copies the files the timelines use into a folder, and writes a copy of the \
                     project there that points at them. The project you are editing is not \
                     changed.",
                );
                if ui.button("Archive Project...").clicked() {
                    self.archive_project(ui.ctx());
                }
                ui.separator();
                self.optimised_media_section(ui);
                if on_wayland() {
                    ui.separator();
                    ui.heading("Drag and Drop");
                    ui.label(
                        "On Wayland, files dragged from the file manager may not arrive at all, with no \
                         message, above all in large batches: the window gets no word of a drop \
                         that failed on the way. If nothing appears in the Media Pool, use \
                         File > Import, or start a session on Xorg.",
                    );
                }
            });
        if !open {
            self.extra.media_management = false;
        }
    }

    /// The window's Optimised Media part: which files have a copy, and the buttons for them.
    fn optimised_media_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Optimised Media");
        ui.label(
            "Small copies of the video files of the media pool that decode quickly, so the \
             timeline and the viewer keep up. They are kept in the data folder and shared by \
             every project; exports read the originals unless the Deliver page says otherwise.",
        );
        let mut use_them = self.use_optimised;
        if ui
            .checkbox(&mut use_them, "Use optimised media")
            .on_hover_text("The viewer plays the copies when there are any")
            .changed()
        {
            self.set_optimised(use_them);
        }

        let videos = self.pool_videos();
        if videos.is_empty() {
            ui.label("The media pool has no video files.");
            return;
        }
        let mut rows: Vec<(String, String, String)> = Vec::new();
        for path in &videos {
            let state = if let Some(done) = self.proxy_jobs.fraction(path) {
                format!("Optimising... {:.0}%", done * 100.0)
            } else if self.proxy_jobs.holds(path) {
                "Waiting".to_string()
            } else if self.proxies.path_for(path).is_some() {
                "Optimised".to_string()
            } else if self.proxy_jobs.is_unsuitable(path) {
                "Read as it is".to_string()
            } else {
                "Not optimised".to_string()
            };
            rows.push((crate::proxy::name_of(path), state, path.clone()));
        }
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                for (name, state, path) in &rows {
                    ui.horizontal(|ui| {
                        ui.label(name).on_hover_text(path);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(state).color(if state == "Optimised" {
                                crate::theme::TEXT_DIM
                            } else {
                                crate::theme::TEXT
                            }));
                        });
                    });
                }
            });

        let waiting = self.optimise_wanted().len();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(waiting > 0, egui::Button::new("Optimise All"))
                .on_hover_text("Makes a copy of every video in the media pool")
                .clicked()
            {
                self.optimise_media();
            }
            if ui
                .add_enabled(self.proxy_jobs.busy(), egui::Button::new("Cancel"))
                .on_hover_text("Stops making copies; the ones already made are kept")
                .clicked()
            {
                self.cancel_proxies();
            }
            let made = rows.iter().any(|(_, state, _)| state == "Optimised");
            if ui
                .add_enabled(made, egui::Button::new("Delete Copies"))
                .on_hover_text("Deletes the copies of this project's video files")
                .clicked()
            {
                self.forget_proxies();
            }
            if ui
                .add_enabled(!self.proxies.copies().is_empty(), egui::Button::new("Delete All"))
                .on_hover_text("Deletes every optimised copy, for every project")
                .clicked()
            {
                self.delete_all_copies();
            }
        });
        if let Some(folder) = self.proxies.folder() {
            ui.label(
                egui::RichText::new(format!("Kept in {}", folder.display()))
                    .color(crate::theme::TEXT_DIM)
                    .size(12.0),
            );
        }
    }

    /// Asks for the folder the archive goes into.
    pub(crate) fn archive_project(&mut self, ctx: &Context) {
        if self.project().media_used().is_empty() {
            self.info("Nothing on the timelines uses a file yet");
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            self.fail("archiving needs the desktop app");
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.info("Choose the folder the archive goes into");
            platform::pick_archive_folder(self.inbox.clone(), ctx.clone());
        }
    }

    /// The folder was chosen: copies the files and writes the project there, in the background.
    pub(crate) fn start_archive(&mut self, folder: String) {
        let archive = match self.archive_plan(Path::new(&folder)) {
            Ok(archive) => archive,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let shown = archive.root.display().to_string();
        let missing = archive.missing.clone();
        self.info(format!(
            "Archiving {} file(s) to {shown}...",
            archive.copies.len()
        ));
        let (inbox, ctx) = (self.inbox.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let (copied, failed) = archive.write(&inbox, &ctx);
            inbox.push(
                &ctx,
                Incoming::Archived {
                    folder: shown,
                    copied,
                    missing,
                    failed,
                },
            );
        });
    }

    /// Says how the archive went.
    pub(crate) fn finish_archive(
        &mut self,
        folder: String,
        copied: usize,
        missing: Vec<String>,
        failed: Vec<String>,
    ) {
        let mut note = format!("Archived {copied} file(s) to {folder}");
        if !missing.is_empty() {
            note.push_str(&format!(
                "; {} file(s) were not found and were not copied",
                missing.len()
            ));
        }
        if !failed.is_empty() {
            note.push_str(&format!("; {} could not be copied", failed.len()));
            self.fail(note);
        } else {
            self.info(note);
        }
    }

    /// The plan for archiving the project into `folder`: a copy with the used files beside it.
    fn archive_plan(&self, folder: &Path) -> Result<Archive, String> {
        let name = crate::autosave::slug(&self.project().name);
        let root = folder.join(&name);
        let media_dir = root.join("media");
        let project_file = root.join(format!("{name}.picasso"));
        // A copy of the project, never the project being edited.
        let mut copy = self.project().clone();
        let mut copies = Vec::new();
        let mut missing = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        let mut new_paths: HashMap<MediaId, String> = HashMap::new();
        for media in self.project().media_used() {
            let Some(item) = self.project().media(media) else {
                continue;
            };
            let source = match &item.source {
                MediaSource::Video { path }
                | MediaSource::Audio { path }
                | MediaSource::Image { path } => path,
                // A title, a solid or a compound clip has no file to copy.
                _ => continue,
            };
            let src = PathBuf::from(source);
            if !src.is_file() {
                missing.push(item.name.clone());
                continue;
            }
            let file_name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("media-{}", media.0));
            let dest = media_dir.join(unique_name(&file_name, &mut taken));
            let dest_path = dest.to_string_lossy().into_owned();
            Command::RelinkMedia {
                media,
                path: dest_path.clone(),
            }
            .apply(&mut copy)
            .map_err(|e| e.to_string())?;
            new_paths.insert(media, dest_path);
            if src != dest {
                copies.push((src, dest));
            }
        }
        let text = copy.to_json().map_err(|e| e.to_string())?;
        // What is known of the media, with the paths it now has.
        let mut facts: Vec<_> = self
            .media_facts
            .iter()
            .filter_map(|(media, fact)| {
                let path = new_paths.get(media)?;
                let mut fact = fact.clone();
                fact.path = path.clone();
                Some(fact)
            })
            .collect();
        facts.sort_by_key(|f| f.media);
        let project = crate::projectfile::encode(&text, &facts)?;
        Ok(Archive {
            root,
            project_file,
            media_dir,
            project,
            copies,
            missing,
        })
    }
}

/// Whether the desktop is a Wayland one, where large drag and drop batches can lose files on the
/// way through XWayland.
fn on_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// A file name no other file in the archive has, adding `-2`, `-3` before the extension.
fn unique_name(file_name: &str, taken: &mut HashSet<String>) -> String {
    if taken.insert(file_name.to_lowercase()) {
        return file_name.to_string();
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".into());
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 2;
    loop {
        let candidate = format!("{stem}-{n}{ext}");
        if taken.insert(candidate.to_lowercase()) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "picasso-archive-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn archiving_in_the_background_writes_the_copy_and_says_so() {
        use picasso_core::Outcome;

        let dir = scratch("background");
        let file = dir.join("clip.mov");
        std::fs::write(&file, b"x").unwrap();

        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        let media = match app.run(Command::AddMedia {
            name: "clip".into(),
            source: MediaSource::Video {
                path: file.to_string_lossy().into_owned(),
            },
            length: Some(10),
            default_duration: 10,
        }) {
            Some(Outcome::Media(id)) => id,
            _ => panic!("the media was not added"),
        };
        app.run(Command::AddClip {
            track: app.project().tracks[0].id,
            media,
            start: 0,
            duration: Some(10),
            with_sound: false,
        });

        let out = dir.join("out");
        let name = crate::autosave::slug(&app.project().name);
        let project_file = out.join(&name).join(format!("{name}.picasso"));
        // The folder is chosen, and the copy is written on its own thread.
        app.start_archive(out.to_string_lossy().into_owned());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            app.process_inbox();
            if project_file.is_file() && app.status.text.starts_with("Archived") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(project_file.is_file(), "the project copy was written");
        assert!(
            app.status.text.starts_with("Archived"),
            "and said so: {}",
            app.status.text
        );
        // The copy points at the file beside it.
        let (text, _) =
            crate::projectfile::decode(std::fs::read(&project_file).unwrap()).unwrap();
        let archived = picasso_core::Project::from_json(&text).unwrap();
        let path = match &archived.media(media).unwrap().source {
            MediaSource::Video { path } => path.clone(),
            other => panic!("the media changed kind: {other:?}"),
        };
        // Compared as paths: where the separators are `\` the text of them differs from `/`.
        assert_eq!(
            PathBuf::from(&path),
            out.join(&name).join("media").join("clip.mov")
        );
        assert!(std::path::Path::new(&path).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_taken_by_another_file_gets_a_number() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name("A001.mov", &mut taken), "A001.mov");
        assert_eq!(unique_name("a001.MOV", &mut taken), "a001-2.MOV");
        assert_eq!(unique_name("A001.mov", &mut taken), "A001-3.mov");
        assert_eq!(unique_name("still.png", &mut taken), "still.png");
    }

    #[test]
    fn the_archive_is_a_copy_that_points_at_the_copied_files() {
        let dir = scratch("copy");
        let source = dir.join("cards");
        std::fs::create_dir_all(&source).unwrap();
        let video = source.join("A001.mov");
        std::fs::write(&video, b"picture").unwrap();
        let image = source.join("still.png");
        std::fs::write(&image, b"png").unwrap();

        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        let video_id = match app.run(Command::AddMedia {
            name: "A001".into(),
            source: MediaSource::Video {
                path: video.to_string_lossy().into_owned(),
            },
            length: Some(100),
            default_duration: 50,
        }) {
            Some(picasso_core::Outcome::Media(id)) => id,
            _ => panic!("the video was not added"),
        };
        let image_id = match app.run(Command::AddMedia {
            name: "still".into(),
            source: MediaSource::Image {
                path: image.to_string_lossy().into_owned(),
            },
            length: None,
            default_duration: 24,
        }) {
            Some(picasso_core::Outcome::Media(id)) => id,
            _ => panic!("the image was not added"),
        };
        // A clip for each, so both count as used.
        app.run(Command::AddClip {
            track: app.project().tracks[0].id,
            media: video_id,
            start: 0,
            duration: Some(50),
            with_sound: false,
        });
        app.run(Command::AddClip {
            track: app.project().tracks[0].id,
            media: image_id,
            start: 50,
            duration: Some(24),
            with_sound: false,
        });

        let out = dir.join("archive");
        let plan = app.archive_plan(&out).unwrap();
        let (copied, failed) = plan.write(&app.inbox, &ctx);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(copied, 2);
        // The files are there, under the project's folder.
        assert!(plan.root.join("media/A001.mov").is_file());
        assert!(plan.root.join("media/still.png").is_file());
        assert!(plan.project_file.is_file());
        // The project copy points at them.
        let (text, _) = crate::projectfile::decode(std::fs::read(&plan.project_file).unwrap())
            .unwrap();
        let archived = picasso_core::Project::from_json(&text).unwrap();
        for media in [video_id, image_id] {
            let source = match &archived.media(media).unwrap().source {
                MediaSource::Video { path } | MediaSource::Image { path } => path,
                other => panic!("the media changed kind: {other:?}"),
            };
            assert!(
                source.starts_with(&plan.root.to_string_lossy().into_owned()),
                "the copy points into the archive: {source}"
            );
            assert!(Path::new(source).is_file());
        }
        // The project being edited was not touched.
        let live = app.project().media(video_id).unwrap();
        assert!(matches!(
            &live.source,
            MediaSource::Video { path } if *path == video.to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_reported_and_the_rest_is_archived() {
        let dir = scratch("missing");
        let here = dir.join("here.mov");
        std::fs::write(&here, b"x").unwrap();

        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        let good = match app.run(Command::AddMedia {
            name: "here".into(),
            source: MediaSource::Video {
                path: here.to_string_lossy().into_owned(),
            },
            length: Some(10),
            default_duration: 10,
        }) {
            Some(picasso_core::Outcome::Media(id)) => id,
            _ => panic!(),
        };
        let gone = match app.run(Command::AddMedia {
            name: "gone".into(),
            source: MediaSource::Video {
                path: dir.join("gone.mov").to_string_lossy().into_owned(),
            },
            length: Some(10),
            default_duration: 10,
        }) {
            Some(picasso_core::Outcome::Media(id)) => id,
            _ => panic!(),
        };
        app.run(Command::AddClip {
            track: app.project().tracks[0].id,
            media: good,
            start: 0,
            duration: Some(10),
            with_sound: false,
        });
        app.run(Command::AddClip {
            track: app.project().tracks[0].id,
            media: gone,
            start: 10,
            duration: Some(10),
            with_sound: false,
        });

        let plan = app.archive_plan(&dir.join("archive")).unwrap();
        assert_eq!(plan.missing, vec!["gone".to_string()]);
        assert_eq!(plan.copies.len(), 1, "only the file that is here is copied");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_files_with_the_same_name_are_both_kept() {
        let dir = scratch("same-name");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("clip.mov"), b"one").unwrap();
        std::fs::write(b.join("clip.mov"), b"two").unwrap();

        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        let mut ids = Vec::new();
        for (folder, name) in [(a.clone(), "one"), (b.clone(), "two")] {
            let id = match app.run(Command::AddMedia {
                name: name.into(),
                source: MediaSource::Video {
                    path: folder.join("clip.mov").to_string_lossy().into_owned(),
                },
                length: Some(10),
                default_duration: 10,
            }) {
                Some(picasso_core::Outcome::Media(id)) => id,
                _ => panic!(),
            };
            ids.push(id);
        }
        app.run(Command::AddClip {
            track: app.project().tracks[0].id,
            media: ids[0],
            start: 0,
            duration: Some(10),
            with_sound: false,
        });
        app.run(Command::AddClip {
            track: app.project().tracks[0].id,
            media: ids[1],
            start: 10,
            duration: Some(10),
            with_sound: false,
        });

        let plan = app.archive_plan(&dir.join("archive")).unwrap();
        assert_eq!(plan.copies.len(), 2);
        let names: Vec<String> = plan
            .copies
            .iter()
            .map(|(_, dest)| dest.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"clip.mov".to_string()));
        assert!(names.contains(&"clip-2.mov".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
