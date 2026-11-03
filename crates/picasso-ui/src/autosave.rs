//! Autosave: while a project has changes that were not saved, a copy of it is written every
//! minute, so a crash or a power cut costs a minute of work instead of the session.
//!
//! A project with a file keeps its autosaves beside it (`shot.picasso.autosave`), where they
//! travel with the project and a copied folder carries them too. A project that was never saved
//! keeps them in the data folder. Three are kept, so one written while the disk is failing is
//! never the only copy.
//!
//! The copy is a whole project file, the same bytes File > Save Project writes, so recovering is
//! opening a project: nothing is half-written and nothing needs a second format.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use egui::Context;

use crate::app::StudioApp;
use crate::fps_prompt::prompt_box;

/// How often an autosave is written while there are changes to save.
pub(crate) const EVERY: Duration = Duration::from_secs(60);

/// How many are kept per project.
const KEEP: usize = 3;

/// What an autosave file is called after the project file: `shot.picasso.autosave`, then
/// `.autosave.1` and `.autosave.2`.
const SUFFIX: &str = ".autosave";

/// An autosave that is newer than the project it belongs to: what was found and what to say.
#[derive(Clone, Debug)]
pub(crate) struct Found {
    /// The autosave file.
    pub file: PathBuf,
    /// The project file it belongs to, when it has one.
    pub project: Option<String>,
    /// When it was written, as a time for the question.
    pub when: String,
}

impl StudioApp {
    /// Writes an autosave when one is due. Called every frame; does nothing unless the project
    /// has changes and a minute has passed since the last one.
    pub(crate) fn tick_autosave(&mut self, ctx: &Context) {
        if !self.session.is_dirty() {
            // Nothing to keep: the timer starts again with the next change.
            self.autosave_at = std::time::Instant::now();
            return;
        }
        let due = EVERY.saturating_sub(self.autosave_at.elapsed());
        if !due.is_zero() {
            // The window only draws when something happens; ask for the frame that autosaves.
            ctx.request_repaint_after(due);
            return;
        }
        if self.playing {
            // Writing the project takes a moment on a large one: not in the middle of playback.
            ctx.request_repaint_after(Duration::from_secs(5));
            return;
        }
        self.autosave_at = std::time::Instant::now();
        let text = match self.project().to_json() {
            Ok(text) => text,
            Err(_) => return,
        };
        let bytes = match self.project_file_bytes(&text) {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        let name = self.project().name.clone();
        let path = self.project_path.as_deref().map(PathBuf::from);
        match write(path.as_deref(), &name, &bytes) {
            Ok(_) => self.autosave_failed = false,
            // Once, not every minute: a full disk would otherwise fill the log.
            Err(e) => {
                if !self.autosave_failed {
                    self.autosave_failed = true;
                    self.fail(format!("could not autosave: {e}"));
                }
            }
        }
    }

    /// Asks to recover an autosave that is newer than the project just opened. Called after a
    /// project is opened; does nothing when there is none, or when the autosave itself is being
    /// opened.
    pub(crate) fn look_for_autosave(&mut self) {
        if self.recovering {
            return;
        }
        let Some(path) = self.project_path.as_deref().map(PathBuf::from) else {
            return;
        };
        if path.to_string_lossy().contains(SUFFIX) {
            return;
        }
        let name = self.project().name.clone();
        let Some(file) = newest(Some(&path), &name) else {
            return;
        };
        // Only when it holds work the project file does not: written after the last save.
        let saved = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let written = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
        let newer = match (saved, written) {
            (Some(saved), Some(written)) => written > saved,
            (None, Some(_)) => true,
            _ => false,
        };
        if !newer {
            return;
        }
        self.recover = Some(Found {
            file,
            project: Some(path.to_string_lossy().into_owned()),
            when: when_text(written),
        });
    }

    /// The same question for a project that was never saved: a copy left behind by a crash.
    #[cfg(not(test))]
    pub(crate) fn look_for_orphan_autosave(&mut self) {
        if self.recovering || !self.project().media.is_empty() || self.session.is_dirty() {
            return;
        }
        let Some(file) = orphan() else {
            return;
        };
        self.recover = Some(Found {
            file: file.clone(),
            project: None,
            when: when_text(std::fs::metadata(&file).and_then(|m| m.modified()).ok()),
        });
    }

    /// Draws "Recover unsaved changes?" while one is waiting, and acts on the answer.
    pub(crate) fn recover_window(&mut self, ctx: &Context) {
        let Some(found) = self.recover.clone() else {
            return;
        };
        let lines = [
            "A copy of this project was kept when the program last stopped.".to_string(),
            format!("It was written {}, and has work the project file does not.", found.when),
        ];
        let answer = prompt_box(
            ctx,
            "recover_prompt",
            "Recover unsaved changes?",
            &lines,
            ("Discard", 330.0),
            "Recover",
        );
        match answer {
            Some(true) => {
                self.recover = None;
                self.recover_from(&found);
            }
            Some(false) => {
                self.recover = None;
                // All the copies go, not only the newest: an older one would ask again. The
                // project file itself is untouched.
                clear_base(&base_of(&found.file));
            }
            None => {}
        }
    }

    /// Opens the autosave, keeping the project's own file as the place it saves to.
    fn recover_from(&mut self, found: &Found) {
        let bytes = match std::fs::read(&found.file) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.fail(format!("could not read the autosave: {e}"));
                return;
            }
        };
        // A copy that cannot be read is not work that can be recovered. Saying it was would
        // hide the failure and leave a project marked as changed that never changed. The file
        // is not deleted either: a later version may read it, and Discard removes it.
        if crate::projectfile::decode(bytes.clone()).is_err() {
            self.fail(format!(
                "{}: the copy kept when the program stopped cannot be read; Discard removes it",
                found.file.display()
            ));
            return;
        }
        // A project that was never saved has no file the copy could be compared with, so once
        // it is open the copy goes: it would be offered again with every start.
        let readable = found.project.is_none();
        let before = self.epoch;
        self.recovering = true;
        self.open_bytes("Recovered project.picasso", found.project.clone(), bytes);
        self.recovering = false;
        if self.epoch == before {
            // It did not open, and the message `open_bytes` left says why.
            return;
        }
        if readable {
            clear_base(&base_of(&found.file));
        }
        // It is now the work being edited: it has changes the file does not.
        self.session.mark_dirty();
        self.info("Recovered the unsaved changes");
    }
}

/// Where a project's autosaves live: beside the project file, or in the data folder when the
/// project has never been saved.
fn base(project: Option<&Path>, name: &str) -> Option<PathBuf> {
    match project {
        Some(path) => Some(PathBuf::from(format!("{}{SUFFIX}", path.display()))),
        // The run's number is in the name: two programs open at once, each with a project
        // called "New Project", would otherwise write over each other's copy.
        None => Some(crate::project_manager::data_dir()?.join("autosave").join(format!(
            "{}-{}{SUFFIX}",
            slug(name),
            std::process::id()
        ))),
    }
}

/// The name as a file name: letters, digits, `-` and `_`, lowercased.
pub(crate) fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "untitled".into() } else { s }
}

/// Slot `i` of a project's autosaves.
fn slot(base: &Path, i: usize) -> PathBuf {
    if i == 0 {
        base.to_path_buf()
    } else {
        PathBuf::from(format!("{}.{i}", base.display()))
    }
}

/// Writes a copy of the project, keeping the older ones behind it. Written to a temporary file
/// first, so a crash while writing never leaves a half copy as the newest one.
pub(crate) fn write(project: Option<&Path>, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let base = base(project, name).ok_or("there is nowhere to keep an autosave")?;
    if let Some(folder) = base.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let tmp = PathBuf::from(format!("{}.tmp", base.display()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    // The oldest goes, the rest step back, and the new one takes the first place.
    let _ = std::fs::remove_file(slot(&base, KEEP - 1));
    for i in (0..KEEP - 1).rev() {
        let _ = std::fs::rename(slot(&base, i), slot(&base, i + 1));
    }
    std::fs::rename(&tmp, &base).map_err(|e| format!("{}: {e}", base.display()))?;
    Ok(base)
}

/// The newest autosave of a project, when there is one.
pub(crate) fn newest(project: Option<&Path>, name: &str) -> Option<PathBuf> {
    let base = base(project, name)?;
    (0..KEEP)
        .map(|i| slot(&base, i))
        .filter_map(|p| {
            let when = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((p, when))
        })
        .max_by_key(|(_, when)| *when)
        .map(|(p, _)| p)
}

/// The newest autosave of a project that was never saved, left behind by a crash.
#[cfg(not(test))]
fn orphan() -> Option<PathBuf> {
    let folder = crate::project_manager::data_dir()?.join("autosave");
    let mut newest: Option<(PathBuf, SystemTime)> = None;
    for entry in std::fs::read_dir(&folder).ok()? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.to_string_lossy().ends_with(SUFFIX) {
            continue;
        }
        let Ok(when) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(_, best)| when > *best) {
            newest = Some((path, when));
        }
    }
    newest.map(|(p, _)| p)
}

/// Deletes a project's autosaves: after a save, or when the changes were given up.
pub(crate) fn clear(project: Option<&Path>, name: &str) {
    let Some(base) = base(project, name) else {
        return;
    };
    clear_base(&base);
}

/// The first place of an autosave: `shot.picasso.autosave.1` is slot 1 of `shot.picasso.autosave`.
fn base_of(file: &Path) -> PathBuf {
    let text = file.to_string_lossy();
    for i in 1..KEEP {
        if let Some(base) = text.strip_suffix(&format!(".{i}")) {
            if base.ends_with(SUFFIX) {
                return PathBuf::from(base);
            }
        }
    }
    file.to_path_buf()
}

/// Deletes every copy kept at `base`.
fn clear_base(base: &Path) {
    let base = base.to_path_buf();
    for i in 0..KEEP {
        let _ = std::fs::remove_file(slot(&base, i));
    }
    let _ = std::fs::remove_file(PathBuf::from(format!("{}.tmp", base.display())));
}

/// How long ago the copy was written, in words. Relative on purpose: a clock time would have to
/// be turned into the reader's own time zone, and this way it reads right in every one of them.
fn when_text(when: Option<SystemTime>) -> String {
    let Some(when) = when else {
        return "the last time it ran".into();
    };
    let ago = SystemTime::now()
        .duration_since(when)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ago_text(ago)
}

/// `secs` seconds ago, in words: "a moment ago", "5 minutes ago", "yesterday", "3 days ago".
fn ago_text(secs: u64) -> String {
    let (n, one, many) = match secs {
        0..60 => return "a moment ago".into(),
        60..3_600 => (secs / 60, "a minute", "minutes"),
        3_600..86_400 => (secs / 3_600, "an hour", "hours"),
        86_400..172_800 => return "yesterday".into(),
        _ => (secs / 86_400, "a day", "days"),
    };
    if n == 1 {
        format!("{one} ago")
    } else {
        format!("{n} {many} ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso-autosave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_project_with_a_file_keeps_its_autosaves_beside_it() {
        let dir = scratch("beside");
        let project = dir.join("shot.picasso");
        let base = base(Some(&project), "shot").unwrap();
        assert_eq!(base, PathBuf::from(format!("{}.autosave", project.display())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_keeps_three_and_the_newest_reads_back() {
        let dir = scratch("three");
        let project = dir.join("shot.picasso");
        std::fs::write(&project, b"the project").unwrap();
        for i in 0..4u8 {
            write(Some(&project), "shot", &[i]).unwrap();
            // A different time for each, so "newest" means something.
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        let latest = newest(Some(&project), "shot").unwrap();
        assert_eq!(std::fs::read(&latest).unwrap(), vec![3]);
        // The three behind it are the last three written, not the first.
        let base = base(Some(&project), "shot").unwrap();
        let kept: Vec<Vec<u8>> = (0..KEEP)
            .filter_map(|i| std::fs::read(slot(&base, i)).ok())
            .collect();
        assert_eq!(kept.len(), 3);
        assert!(kept.contains(&vec![1]) && kept.contains(&vec![2]));
        assert!(!kept.contains(&vec![0]), "the oldest went");
        clear(Some(&project), "shot");
        assert!(newest(Some(&project), "shot").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_never_saved_keeps_its_autosave_in_the_data_folder() {
        let base = base(None, "My Untitled Project").unwrap();
        let text = base.to_string_lossy().into_owned();
        assert!(text.contains("my_untitled_project-"), "named for the project and this run");
        assert!(text.ends_with(".autosave"));
        assert_eq!(text, crate::autosave::base(None, "My Untitled Project").unwrap().to_string_lossy());
        assert!(base.to_string_lossy().contains("autosave"));
    }

    #[test]
    fn discarding_removes_every_copy_not_only_the_newest() {
        let dir = scratch("discard");
        let project = dir.join("shot.picasso");
        for i in 0..3u8 {
            write(Some(&project), "shot", &[i]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        let newest = newest(Some(&project), "shot").unwrap();
        // From any slot, the first place is found again.
        let base = base(Some(&project), "shot").unwrap();
        assert_eq!(base_of(&newest), base);
        assert_eq!(base_of(&slot(&base, 2)), base);
        clear_base(&base_of(&newest));
        assert!(
            (0..KEEP).all(|i| !slot(&base, i).exists()),
            "no older copy is left to ask again"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_name_is_made_safe_for_a_file_name() {
        assert_eq!(slug("New Project 1"), "new_project_1");
        assert_eq!(slug("../../etc/passwd"), "etc_passwd");
        assert_eq!(slug("   "), "untitled");
    }

    #[test]
    fn the_question_says_how_long_ago_in_words() {
        // A clock time would have to be turned into the reader's time zone; these cannot be wrong.
        assert_eq!(ago_text(0), "a moment ago");
        assert_eq!(ago_text(59), "a moment ago");
        assert_eq!(ago_text(60), "a minute ago");
        assert_eq!(ago_text(119), "a minute ago");
        assert_eq!(ago_text(120), "2 minutes ago");
        assert_eq!(ago_text(3_600), "an hour ago");
        assert_eq!(ago_text(7_200), "2 hours ago");
        assert_eq!(ago_text(86_400), "yesterday");
        assert_eq!(ago_text(172_800), "2 days ago");
        assert_eq!(ago_text(30 * 86_400), "30 days ago");
        // A file from the future (a clock that moved) still reads as something.
        assert_eq!(when_text(Some(SystemTime::now() + Duration::from_secs(60))), "a moment ago");
        assert_eq!(when_text(None), "the last time it ran");
    }

    #[test]
    fn a_change_kept_while_the_program_ran_is_offered_when_it_opens_again() {
        use picasso_core::Command;

        let dir = scratch("roundtrip");
        let project = dir.join("shot.picasso");
        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        app.load_demo();
        app.project_path = Some(project.to_string_lossy().into_owned());
        // The project file as it was last saved.
        std::fs::write(&project, b"{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        // A change, and an autosave due.
        app.run(Command::RenameProject {
            name: "Renamed".into(),
        });
        app.autosave_at = std::time::Instant::now() - EVERY - Duration::from_secs(1);
        app.tick_autosave(&ctx);
        assert!(
            newest(Some(&project), "Renamed").is_some(),
            "the change was kept beside the project"
        );
        // Opening the project again asks about it.
        let mut again = StudioApp::new(&ctx);
        again.load_demo();
        again.project_path = Some(project.to_string_lossy().into_owned());
        again.look_for_autosave();
        assert!(again.recover.is_some(), "the copy is offered");
        // The project file itself was not touched.
        assert_eq!(std::fs::read(&project).unwrap(), b"{}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovering_opens_the_copy_and_keeps_the_project_file_as_the_place_it_saves() {
        use picasso_core::Command;

        let dir = scratch("recover");
        let project = dir.join("shot.picasso");
        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        app.load_demo();
        app.project_path = Some(project.to_string_lossy().into_owned());
        std::fs::write(&project, b"{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.run(Command::RenameProject {
            name: "Recovered".into(),
        });
        app.autosave_at = std::time::Instant::now() - EVERY - Duration::from_secs(1);
        app.tick_autosave(&ctx);

        // A fresh program opens the project and is offered the copy.
        let mut again = StudioApp::new(&ctx);
        again.project_path = Some(project.to_string_lossy().into_owned());
        again.look_for_autosave();
        let found = again.recover.clone().expect("the copy is offered");
        again.recover_from(&found);

        // The work is back, and it is unsaved work on the project's own file.
        assert_eq!(again.project().name, "Recovered");
        assert!(again.session.is_dirty(), "the file does not have it yet");
        assert_eq!(
            again.project_path.as_deref(),
            Some(project.to_string_lossy().as_ref())
        );
        // The project file itself still has the old bytes.
        assert_eq!(std::fs::read(&project).unwrap(), b"{}");

        // Saving is what clears the copies (the inbox arm does this).
        clear(Some(&project), "Recovered");
        assert!(newest(Some(&project), "Recovered").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_that_cannot_be_read_is_not_reported_as_recovered() {
        let dir = scratch("corrupt");
        let project = dir.join("shot.picasso");
        std::fs::write(&project, b"{}").unwrap();
        let file = base(Some(&project), "shot").unwrap();
        std::fs::write(&file, b"not a project at all").unwrap();

        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        app.load_demo();
        app.project_path = Some(project.to_string_lossy().into_owned());
        let found = Found {
            file: file.clone(),
            project: Some(project.to_string_lossy().into_owned()),
            when: "a moment ago".into(),
        };
        let epoch = app.epoch;
        app.recover_from(&found);

        assert!(app.status.error, "it says it failed: {}", app.status.text);
        assert_eq!(app.epoch, epoch, "nothing was opened");
        assert!(!app.session.is_dirty(), "and no change is invented");
        assert!(file.is_file(), "the copy is left for Discard to remove");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_autosave_older_than_the_project_is_not_offered() {
        let dir = scratch("stale");
        let project = dir.join("shot.picasso");
        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        app.load_demo();
        app.project_path = Some(project.to_string_lossy().into_owned());
        app.run(picasso_core::Command::RenameProject {
            name: "Renamed".into(),
        });
        app.autosave_at = std::time::Instant::now() - EVERY - Duration::from_secs(1);
        app.tick_autosave(&ctx);
        // Saved since: the project file is newer than the copy.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&project, b"{}").unwrap();
        let mut again = StudioApp::new(&ctx);
        again.project_path = Some(project.to_string_lossy().into_owned());
        again.look_for_autosave();
        assert!(again.recover.is_none(), "a saved project asks nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_autosave_from_a_crash_is_refused_not_panicked_on() {
        let dir = scratch("truncated");
        let project = dir.join("shot.picasso");
        std::fs::write(&project, b"{}").unwrap();
        // Simulate: the autosave was being written, then the process was killed mid-write.
        // The file is valid JSON syntax but truncated structurally — half a project.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let autosave = base(Some(&project), "shot").unwrap();
        let good = StudioApp::new(&egui::Context::default());
        let good_json = good.project().to_json().unwrap_or_default();
        let half = good_json.len() / 2;
        std::fs::write(&autosave, &good_json[..half]).unwrap();

        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        app.load_demo();
        app.project_path = Some(project.to_string_lossy().into_owned());
        app.look_for_autosave();
        // The truncated copy is too new to ignore, so recovery is offered — but recovering
        // from it must fail gracefully, not panic.
        let found = app.recover.clone().expect("truncated copy is newer than the project");
        let epoch = app.epoch;
        app.recover_from(&found);
        assert!(app.status.error, "truncated copy should be refused: {}", app.status.text);
        assert_eq!(app.epoch, epoch, "the project was not changed");
        // The copy is left on disk for Discard to remove.
        assert!(autosave.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
