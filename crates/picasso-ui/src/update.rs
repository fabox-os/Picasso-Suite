//! Help > Check for Updates: asks GitHub for the latest release and installs it the way this copy
//! was installed.
//!
//! - Windows (the release zip, with FFmpeg and its licence beside the program): the new zip is unpacked
//!   over the folder. A running program cannot be overwritten but can be renamed, so each file in
//!   use is renamed to `*.old` first; the next start deletes those.
//! - macOS (`Picasso Suite.app`): the new app replaces the old one, which is kept as
//!   `Picasso Suite.app.old` until the next start.
//! - Flatpak: the new bundle is installed by the host's `flatpak`, through `flatpak-spawn`.
//! - AppImage: the new file is downloaded beside the old one and renamed over it.
//! - Debian/Ubuntu and Fedora/openSUSE (`/usr/bin`): the new package is installed with `pkexec`,
//!   which asks for the password.
//! - Anything else (a build from source, or a copy made by hand): the package for this computer
//!   (the AppImage, or the Windows or macOS zip) is saved to the Downloads folder, to run from there.
//!   The Flathub build is updated by Flathub, so nothing is downloaded for it.
//!
//! The network work goes through the system's `curl`, so the program carries no HTTP or TLS code
//! of its own. Packages are only downloaded from this repository's release files.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use egui::Context;

/// The GitHub repository the releases come from.
const REPO: &str = "fabox-os/Picasso-Suite";
/// The release page, opened when this copy cannot update itself.
pub(crate) const RELEASES_PAGE: &str = "https://github.com/fabox-os/Picasso-Suite/releases/latest";
/// Release files are only downloaded from here.
const DOWNLOAD_PREFIX: &str = "https://github.com/fabox-os/Picasso-Suite/releases/download/";
/// The longest release notes shown in the window.
const MAX_NOTES: usize = 6000;
/// What to do with a package saved to Downloads for a copy that was not installed (see
/// [`Install::Downloads`]).
#[cfg(windows)]
const HOW_TO_RUN: &str = "Unzip it and start Picasso Suite from the new folder.";
#[cfg(target_os = "macos")]
const HOW_TO_RUN: &str = "Unzip it and move the app to Applications.";
#[cfg(not(any(windows, target_os = "macos")))]
const HOW_TO_RUN: &str = "Start the AppImage to open it.";

/// How this copy of the program was installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Install {
    /// The Windows release zip, unpacked in this folder.
    WindowsZip(PathBuf),
    /// The macOS app at this path.
    MacApp(PathBuf),
    Flatpak,
    /// The AppImage file at this path (its runtime says where, in `APPIMAGE`).
    AppImage(PathBuf),
    Deb,
    Rpm,
    /// Built from source or copied by hand: the package for this computer is saved to this folder
    /// (Downloads), to run from there.
    Downloads(PathBuf),
    /// The Flathub build: Flathub updates it (`flatpak update`), so nothing is installed here.
    Unmanaged,
}

impl Install {
    /// How this copy was installed, from where the program runs.
    pub(crate) fn detect() -> Self {
        if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists() {
            // The Flathub build is updated by Flathub (`flatpak update`), not by this program: it
            // has no right to run the host's flatpak.
            if Path::new("/app/share/picasso-suite/updates-by-flathub").exists() {
                return Self::Unmanaged;
            }
            return Self::Flatpak;
        }
        if cfg!(target_os = "linux") {
            if let Some(file) = std::env::var_os("APPIMAGE")
                .map(PathBuf::from)
                .filter(|p| p.is_file())
            {
                return Self::AppImage(file);
            }
        }
        let Ok(exe) = std::env::current_exe() else {
            return Self::by_hand();
        };
        if cfg!(windows) {
            // The release zip carries FFmpeg and its licence beside the program.
            return match exe.parent() {
                Some(dir) if has_ffmpeg_beside(dir) => Self::WindowsZip(dir.into()),
                _ => Self::by_hand(),
            };
        }
        if cfg!(target_os = "macos") {
            return exe
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e == "app"))
                .map_or_else(Self::by_hand, |app| Self::MacApp(app.into()));
        }
        if exe.starts_with("/usr") {
            // dpkg's own list of the package's files, so an rpm install on a system that also
            // has dpkg (or the other way round) is not mistaken.
            if Path::new("/var/lib/dpkg/info/picasso-suite.list").exists() {
                return Self::Deb;
            }
            if Path::new("/usr/bin/rpm").exists() {
                return Self::Rpm;
            }
            if Path::new("/usr/bin/dpkg").exists() {
                return Self::Deb;
            }
        }
        Self::by_hand()
    }

    /// A copy that was not installed from a package: its packages go to Downloads.
    fn by_hand() -> Self {
        Self::Downloads(downloads_dir())
    }

    /// Whether the new version can be installed, or saved for the user to run, from inside the program.
    pub(crate) fn can_update(&self) -> bool {
        *self != Self::Unmanaged
    }

    /// Whether a release file is this kind of package.
    fn wants(&self, file: &str) -> bool {
        match self {
            Self::WindowsZip(_) => file.ends_with("-windows.zip"),
            Self::MacApp(_) => {
                file.ends_with("-macos-universal.zip")
                    || (cfg!(target_arch = "aarch64") && file.ends_with("-macos-arm64.zip"))
            }
            // A copy made by hand gets the package for the system it runs on.
            Self::Downloads(_) if cfg!(windows) => file.ends_with("-windows.zip"),
            Self::Downloads(_) if cfg!(target_os = "macos") => {
                file.ends_with("-macos-universal.zip")
                    || (cfg!(target_arch = "aarch64") && file.ends_with("-macos-arm64.zip"))
            }
            Self::Downloads(_) => file.ends_with(&format!("-{}.AppImage", std::env::consts::ARCH)),
            // The ARM64 bundle is `...-aarch64.flatpak`; the x86_64 one has no processor in its name.
            Self::Flatpak if cfg!(target_arch = "aarch64") => file.ends_with("-aarch64.flatpak"),
            Self::Flatpak => file.ends_with(".flatpak") && !file.ends_with("-aarch64.flatpak"),
            // Linux packages are built for x86_64 and aarch64; Debian calls them amd64 and arm64.
            Self::AppImage(_) => file.ends_with(&format!("-{}.AppImage", std::env::consts::ARCH)),
            Self::Deb if cfg!(target_arch = "aarch64") => file.ends_with("arm64.deb"),
            Self::Deb => file.ends_with("amd64.deb"),
            Self::Rpm => file.ends_with(&format!("{}.rpm", std::env::consts::ARCH)),
            Self::Unmanaged => false,
        }
    }
}

/// The user's Downloads folder (the home folder when there is none, the temporary folder without one).
fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    match home {
        Some(home) => {
            let downloads = home.join("Downloads");
            if downloads.is_dir() {
                downloads
            } else {
                home
            }
        }
        None => std::env::temp_dir(),
    }
}

/// Whether `dir` is an unpacked release zip: those carry FFmpeg's licence beside the program. A
/// build from source has FFmpeg's DLLs copied beside it too (see `apps/studio/build.rs`), but not
/// the licence, so it is never overwritten by an update.
fn has_ffmpeg_beside(dir: &Path) -> bool {
    dir.join("FFMPEG-LICENSE.txt").is_file()
}

/// A release on GitHub.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Release {
    pub tag: String,
    pub notes: String,
    /// The package for this kind of install: its file name and download address.
    pub package: Option<(String, String)>,
}

/// This build's version: the release tag it was built from when the build says so
/// (`PICASSO_RELEASE_TAG`), else the version in `Cargo.toml`.
pub(crate) fn current_version() -> String {
    option_env!("PICASSO_RELEASE_TAG")
        .map(|t| t.trim_start_matches('v').to_string())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// `v0.2.0-hotfix` as its numbers and the words after them: `([0, 2, 0], "hotfix")`.
fn parse_version(tag: &str) -> ([u64; 3], String) {
    let tag = tag.trim().trim_start_matches(['v', 'V']);
    let (numbers, suffix) = tag.split_once('-').unwrap_or((tag, ""));
    let mut out = [0u64; 3];
    for (slot, part) in out.iter_mut().zip(numbers.split('.')) {
        *slot = part.trim().parse().unwrap_or(0);
    }
    (out, suffix.to_ascii_lowercase())
}

/// Whether release `latest` comes after version `current`. A hotfix of a version (`0.2.0-hotfix`)
/// comes after the version itself (`0.2.0`), as this project tags them.
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    let (a, a_suffix) = parse_version(latest);
    let (b, b_suffix) = parse_version(current);
    if a != b {
        return a > b;
    }
    match (a_suffix.is_empty(), b_suffix.is_empty()) {
        (false, true) => true,
        (false, false) => a_suffix > b_suffix,
        _ => false,
    }
}

/// Reads GitHub's answer about the latest release; the package is the one for `install`.
fn parse_release(json: &str, install: &Install) -> Result<Release, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("GitHub's answer could not be read: {e}"))?;
    let tag = v["tag_name"]
        .as_str()
        .ok_or("GitHub did not name a release")?
        .to_string();
    let mut notes = v["body"].as_str().unwrap_or("").replace("\r\n", "\n");
    if notes.len() > MAX_NOTES {
        let mut cut = MAX_NOTES;
        while !notes.is_char_boundary(cut) {
            cut -= 1;
        }
        notes.truncate(cut);
        notes.push_str("\n...");
    }
    let package = v["assets"].as_array().and_then(|assets| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            // The name is put in a path: only a plain file name is taken.
            if Path::new(name).file_name() != Some(std::ffi::OsStr::new(name)) || name.starts_with('.') {
                return None;
            }
            let url = a["browser_download_url"].as_str()?;
            (install.wants(name) && url.starts_with(DOWNLOAD_PREFIX))
                .then(|| (name.to_string(), url.to_string()))
        })
    });
    Ok(Release {
        tag,
        notes,
        package,
    })
}

/// A program run with no console window popping up on Windows.
fn quiet(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs `cmd` and says what went wrong in words when it fails.
fn run(mut cmd: Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("{what}: could not start it ({e})"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    Err(if err.is_empty() {
        format!("{what} failed ({})", out.status)
    } else {
        format!("{what} failed: {err}")
    })
}

/// Asks GitHub for the latest release.
fn fetch_latest(install: &Install) -> Result<Release, String> {
    let mut cmd = quiet("curl");
    cmd.args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-time", "20"])
        .args(["-H", "Accept: application/vnd.github+json"])
        .args(["-H", "User-Agent: picasso-suite-updater"])
        .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let body = run(cmd, "Asking GitHub for the latest release")?;
    parse_release(&String::from_utf8_lossy(&body), install)
}

/// Downloads `url` (a file of this repository's releases) to `dest`.
fn download(url: &str, dest: &Path) -> Result<(), String> {
    if !url.starts_with(DOWNLOAD_PREFIX) {
        return Err("the package is not on this project's release page".into());
    }
    let mut cmd = quiet("curl");
    cmd.args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-time", "900", "-o"])
        .arg(dest)
        .arg(url);
    run(cmd, "Downloading the new version").map(|_| ())
}

/// An empty folder for this update's files.
fn work_dir(tag: &str) -> Result<PathBuf, String> {
    let safe: String = tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    // The package is installed (with pkexec, as root, for a .deb or .rpm) after it is downloaded here, so
    // nobody else may be able to put another file in its place in between: the folder has a name
    // nobody can guess and is made for this user alone, never a name another user could have made.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let temp = std::env::temp_dir();
    let dir = temp.join(format!("picasso-suite-update-{safe}-{}-{nonce:x}", std::process::id()));
    // What an earlier update left (a real folder; a link is not followed).
    if let Ok(entries) = std::fs::read_dir(&temp) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink());
            if is_dir && name.starts_with("picasso-suite-update-") && entry.path() != dir {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        // `create` (not `create_dir_all`) fails when the name exists, so a folder made by someone else is never used.
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// The folder (at most two levels down) that holds `file`.
fn folder_with(root: &Path, file: &str) -> Option<PathBuf> {
    if root.join(file).exists() {
        return Some(root.into());
    }
    let entries = std::fs::read_dir(root).ok()?;
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .find_map(|p| folder_with(&p, file))
}

/// What happened, for the window.
pub(crate) struct Installed {
    pub message: String,
    /// Whether starting the program again shows the new version.
    pub restart: bool,
}

/// Where package `file` is put before it is installed. Flatpak and the Linux packages are
/// installed by programs outside this one, which must be able to read the file: inside a Flatpak
/// the cache folder is shared with the host.
fn package_path(install: &Install, file: &str, work: &Path) -> PathBuf {
    match install {
        Install::Flatpak => {
            let cache = std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let _ = std::fs::create_dir_all(&cache);
            cache.join(file)
        }
        // Beside the AppImage, so it can be renamed over it (a rename cannot cross drives).
        Install::AppImage(target) => target.with_file_name(format!(".{file}.download")),
        // Saved to Downloads itself: the user keeps the package there.
        Install::Downloads(dir) => {
            let _ = std::fs::create_dir_all(dir);
            dir.join(file)
        }
        _ => work.join(file),
    }
}

/// Downloads `release`'s package and installs it over this copy.
fn download_and_install(
    install: &Install,
    release: &Release,
    step: &dyn Fn(&str),
) -> Result<Installed, String> {
    let (file, url) = release
        .package
        .clone()
        .ok_or("this release has no package for this kind of install")?;
    let work = work_dir(&release.tag)?;
    let package = package_path(install, &file, &work);
    step(&format!("Downloading {file}..."));
    if let Err(e) = download(&url, &package) {
        // curl leaves what it had when the connection broke.
        let _ = std::fs::remove_file(&package);
        return Err(e);
    }
    step("Installing...");
    install_package(install, &package, &work, &release.tag)
}

/// `picasso-suite --update-from <package>`: installs a release package that is already on this
/// computer over this copy, the way Check for Updates installs the one it downloads (the same
/// code from here on). The release workflow uses it to try the updater where each kind of
/// package really runs. The file must be this kind of install's package.
pub fn update_from_file(path: &Path) -> Result<String, String> {
    let install = Install::detect();
    if !install.can_update() {
        return Err(
            "this copy was not installed from a release package, so it cannot update itself".into(),
        );
    }
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the package has no file name")?
        .to_string();
    if !install.wants(&file) {
        return Err(format!(
            "{file} is not a package for this copy ({install:?})"
        ));
    }
    let work = work_dir("from-file")?;
    let package = package_path(&install, &file, &work);
    // Copied to where a download would be, unless it is there already.
    let same = matches!(
        (path.canonicalize(), package.canonicalize()),
        (Ok(a), Ok(b)) if a == b
    );
    if !same {
        std::fs::copy(path, &package).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    install_package(&install, &package, &work, &file)?;
    Ok(format!("{file} is installed over this copy ({install:?})"))
}

/// This build's version, and the commit it was built from when the build says so
/// (`PICASSO_BUILD_COMMIT`): what `picasso-suite --version` prints.
pub fn version_line() -> String {
    let commit = option_env!("PICASSO_BUILD_COMMIT")
        .map(|c| format!(" ({})", &c[..c.len().min(12)]))
        .unwrap_or_default();
    format!("picasso-suite {}{commit}", current_version())
}

/// Installs `package`, which is in place for this kind of install ([`package_path`]), over this
/// copy; `work` is this update's folder, `tag` names the new version in the messages.
fn install_package(
    install: &Install,
    package: &Path,
    work: &Path,
    tag: &str,
) -> Result<Installed, String> {
    let done = match install {
        Install::WindowsZip(dir) => install_windows(package, work, dir)?,
        Install::MacApp(app) => install_mac(package, work, app)?,
        Install::Flatpak => {
            let mut cmd = quiet("flatpak-spawn");
            cmd.args(["--host", "flatpak", "install", "--noninteractive", "-y", "--reinstall"]);
            // A per-user install is replaced per-user; flatpak would otherwise install it for
            // the whole system, asking for a password, next to the old one.
            let info = std::fs::read_to_string("/.flatpak-info").unwrap_or_default();
            let user = is_user_flatpak(&info);
            if user {
                cmd.arg("--user");
            }
            cmd.arg(package);
            run(cmd, "Installing the Flatpak").map_err(|e| {
                format!(
                    "{e}\n\nInstall it from a terminal instead:\nflatpak install {}--reinstall {}",
                    if user { "--user " } else { "" },
                    package.display()
                )
            })?;
            Installed {
                message: format!(
                    "Picasso Suite {tag} is installed. Close Picasso Suite and open it again."
                ),
                restart: false,
            }
        }
        Install::AppImage(target) => install_appimage(package, target, tag)?,
        Install::Deb | Install::Rpm => install_linux_package(install, package, tag)?,
        Install::Downloads(dir) => {
            let name = package
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.ends_with(".AppImage") {
                make_runnable(package).map_err(|e| format!("{}: {e}", package.display()))?;
            }
            Installed {
                message: format!(
                    "Picasso Suite {tag} is saved as {name} in {}. {}",
                    dir.display(),
                    HOW_TO_RUN
                ),
                restart: false,
            }
        }
        Install::Unmanaged => return Err("this copy cannot update itself".into()),
    };
    if !matches!(install, Install::Flatpak | Install::Deb | Install::Rpm) {
        let _ = std::fs::remove_dir_all(work);
    }
    Ok(done)
}

/// Unpacks the Windows zip and puts its files in `dir`, renaming the files in use first.
fn install_windows(zip: &Path, work: &Path, dir: &Path) -> Result<Installed, String> {
    let unpacked = work.join("unpacked");
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    // Windows 10 and later carry bsdtar, which reads zip files. It is named by its full path: a
    // GNU tar (from Git, say) earlier on the PATH cannot.
    let tar = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("tar"));
    let mut cmd = quiet(tar);
    cmd.arg("-xf").arg(zip).arg("-C").arg(&unpacked);
    run(cmd, "Unpacking the new version")?;
    let source = folder_with(&unpacked, "picasso-suite.exe")
        .ok_or("the new version's zip has no picasso-suite.exe")?;
    let entries = std::fs::read_dir(&source).map_err(|e| e.to_string())?;
    // What was done, so a failure half way can put the old version back: (new file, the old
    // file renamed out of its way, if there was one).
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let undo = |done: &[(PathBuf, Option<PathBuf>)]| {
        for (to, old) in done.iter().rev() {
            let _ = std::fs::remove_file(to);
            if let Some(old) = old {
                let _ = std::fs::rename(old, to);
            }
        }
    };
    for entry in entries.flatten() {
        let from = entry.path();
        if !from.is_file() {
            continue;
        }
        let to = dir.join(entry.file_name());
        let mut old = None;
        if to.exists() {
            let mut name = to.as_os_str().to_owned();
            name.push(".old");
            let renamed = PathBuf::from(name);
            let _ = std::fs::remove_file(&renamed);
            if let Err(e) = std::fs::rename(&to, &renamed) {
                undo(&done);
                return Err(format!(
                    "{} cannot be replaced ({e}). If Picasso Suite is in Program Files, move its \
                     folder somewhere you can write to, or update it by hand.",
                    to.display()
                ));
            }
            old = Some(renamed);
        }
        let copied = std::fs::copy(&from, &to);
        done.push((to.clone(), old));
        if let Err(e) = copied {
            undo(&done);
            return Err(format!(
                "{}: {e}. The version you had is kept.",
                to.display()
            ));
        }
    }
    Ok(Installed {
        message: "The new version is installed. Restart Picasso Suite to use it.".into(),
        restart: true,
    })
}

/// Unpacks the macOS zip and puts the new app where `app` is.
fn install_mac(zip: &Path, work: &Path, app: &Path) -> Result<Installed, String> {
    let unpacked = work.join("unpacked");
    let mut cmd = quiet("ditto");
    cmd.args(["-x", "-k"]).arg(zip).arg(&unpacked);
    run(cmd, "Unpacking the new version")?;
    let new_app = std::fs::read_dir(&unpacked)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or("the new version's zip has no app")?;
    let old = old_app_path(app);
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(app, &old).map_err(|e| format!("{} cannot be replaced: {e}", app.display()))?;
    let mut cmd = quiet("ditto");
    cmd.arg(&new_app).arg(app);
    if let Err(e) = run(cmd, "Copying the new app") {
        // Put the old app back so the program still starts.
        let _ = std::fs::remove_dir_all(app);
        let _ = std::fs::rename(&old, app);
        return Err(e);
    }
    Ok(Installed {
        message: "The new version is installed. Restart Picasso Suite to use it.".into(),
        restart: true,
    })
}

fn old_app_path(app: &Path) -> PathBuf {
    let mut old = app.as_os_str().to_owned();
    old.push(".old");
    PathBuf::from(old)
}

/// Makes the downloaded AppImage runnable and renames it over the old one. The running copy keeps
/// reading the old file until it closes.
fn install_appimage(new: &Path, target: &Path, tag: &str) -> Result<Installed, String> {
    let replaced = make_runnable(new).and_then(|()| std::fs::rename(new, target));
    if let Err(e) = replaced {
        let _ = std::fs::remove_file(new);
        return Err(format!(
            "{} cannot be replaced ({e}). Download the new AppImage from the release page instead.",
            target.display()
        ));
    }
    Ok(Installed {
        message: format!("Picasso Suite {tag} is installed. Restart Picasso Suite to use it."),
        restart: true,
    })
}

#[cfg(unix)]
fn make_runnable(file: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_runnable(_file: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Whether the Flatpak runs from a per-user installation, from its `/.flatpak-info`: the app's
/// files are then under the user's `.local/share/flatpak`.
fn is_user_flatpak(info: &str) -> bool {
    info.lines()
        .filter_map(|l| l.trim().strip_prefix("app-path="))
        .any(|path| path.contains("/.local/share/flatpak/"))
}

/// Whether this program runs as root (Linux): it then needs no password to install a package.
fn running_as_root() -> bool {
    // `Uid:` lists the real, effective, saved and file-system ids; the effective one counts.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|l| l.strip_prefix("Uid:"))
                .map(|ids| ids.split_whitespace().nth(1) == Some("0"))
        })
        .unwrap_or(false)
}

/// The package manager and its arguments that install a `.deb` or `.rpm` file.
fn package_manager(install: &Install) -> (&'static str, &'static [&'static str]) {
    match install {
        Install::Deb => ("apt-get", &["install", "-y"]),
        _ if Path::new("/usr/bin/dnf").exists() => ("dnf", &["install", "-y"]),
        // The release's rpm is not signed; zypper refuses it unless told.
        _ => (
            "zypper",
            &["--non-interactive", "install", "--allow-unsigned-rpm"],
        ),
    }
}

/// Installs a `.deb` or `.rpm` with the system's package manager, asking for the password
/// (through `pkexec`) unless this program already runs as root.
fn install_linux_package(install: &Install, package: &Path, tag: &str) -> Result<Installed, String> {
    let manual = match install {
        Install::Deb => format!("sudo apt install {}", package.display()),
        _ => format!("sudo dnf install {}", package.display()),
    };
    let (program, args) = package_manager(install);
    let mut cmd = if running_as_root() {
        quiet(program)
    } else {
        let mut c = quiet("pkexec");
        c.arg(program);
        c
    };
    cmd.args(args).arg(package);
    run(cmd, "Installing the package")
        .map_err(|e| format!("{e}\n\nInstall it from a terminal instead:\n{manual}"))?;
    Ok(Installed {
        message: format!("Picasso Suite {tag} is installed. Restart Picasso Suite to use it."),
        restart: true,
    })
}

/// Deletes what the last update left behind: `*.old` files beside the program on Windows, the old
/// app on macOS. Run at start.
pub(crate) fn clean_up() {
    match Install::detect() {
        Install::WindowsZip(dir) => clean_up_dir(&dir),
        Install::MacApp(app) => {
            let _ = std::fs::remove_dir_all(old_app_path(&app));
        }
        _ => {}
    }
}

/// Deletes the `*.old` files an update left in `dir`.
fn clean_up_dir(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().ends_with(".old") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Opens `url` in the web browser.
pub(crate) fn open_in_browser(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = quiet("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        quiet("open")
    } else {
        quiet("xdg-open")
    };
    let _ = cmd.arg(url).spawn();
}

/// Starts this program again (the new version, once installed).
fn relaunch(install: &Install) {
    match install {
        Install::MacApp(app) => {
            let _ = quiet("open").arg("-n").arg(app).spawn();
        }
        // The program itself runs from a folder the AppImage mounts, which goes away on exit.
        Install::AppImage(file) => {
            let _ = quiet(file).spawn();
        }
        _ => {
            if let Ok(exe) = std::env::current_exe() {
                let _ = quiet(exe).spawn();
            }
        }
    }
}

/// Where the check or the update is.
#[derive(Clone, Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate(String),
    Available(Release),
    Working(String),
    Done {
        message: String,
        restart: bool,
    },
    Failed(String),
}

/// The Check for Updates window and the work behind it.
#[derive(Default)]
pub(crate) struct Updater {
    pub open: bool,
    phase: Arc<Mutex<Phase>>,
    install: Option<Install>,
}

impl Updater {
    fn phase(&self) -> Phase {
        self.phase.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn install(&mut self) -> Install {
        self.install.get_or_insert_with(Install::detect).clone()
    }

    /// Opens the window and asks GitHub for the latest release.
    pub(crate) fn check(&mut self, ctx: &Context) {
        self.open = true;
        if matches!(self.phase(), Phase::Checking | Phase::Working(_)) {
            return;
        }
        let install = self.install();
        let shared = self.phase.clone();
        let ctx = ctx.clone();
        *shared.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Checking;
        std::thread::spawn(move || {
            let current = current_version();
            let phase = match fetch_latest(&install) {
                Ok(r) if is_newer(&r.tag, &current) => Phase::Available(r),
                Ok(r) => Phase::UpToDate(r.tag),
                Err(e) => Phase::Failed(e),
            };
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = phase;
            ctx.request_repaint();
        });
    }

    fn update(&mut self, ctx: &Context, release: Release) {
        let install = self.install();
        let shared = self.phase.clone();
        let ctx = ctx.clone();
        *shared.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Working("Starting...".into());
        std::thread::spawn(move || {
            let step = |s: &str| {
                *shared.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Working(s.into());
                ctx.request_repaint();
            };
            let phase = match download_and_install(&install, &release, &step) {
                Ok(done) => Phase::Done {
                    message: done.message,
                    restart: done.restart,
                },
                Err(e) => Phase::Failed(e),
            };
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = phase;
            ctx.request_repaint();
        });
    }

    /// Draws the window while it is open.
    pub(crate) fn window(&mut self, ctx: &Context) {
        if !self.open {
            return;
        }
        let phase = self.phase();
        let install = self.install();
        let current = current_version();
        let mut open = true;
        let mut wanted: Option<Release> = None;
        let mut check_again = false;
        let mut restart = false;
        egui::Window::new("Check for Updates")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                match &phase {
                    Phase::Idle | Phase::Checking => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Checking GitHub for a newer version...");
                        });
                    }
                    Phase::UpToDate(latest) => {
                        ui.label(format!("Picasso Suite {current} is up to date."));
                        ui.weak(format!("The latest release is {latest}."));
                    }
                    Phase::Available(release) => {
                        ui.heading(format!("Picasso Suite {} is available", release.tag));
                        ui.label(format!("You have {current}."));
                        if !release.notes.trim().is_empty() {
                            ui.add_space(4.0);
                            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                ui.label(&release.notes);
                            });
                        }
                        ui.add_space(6.0);
                        if let Install::Downloads(dir) = &install {
                            ui.label(format!(
                                "This copy was not installed from a package, so the new version \
                                 is saved to {} for you to run.",
                                dir.display()
                            ));
                        } else if !install.can_update() {
                            ui.label(
                                "This copy is updated by Flathub, so it cannot update itself. \
                                 Download the new version from the release page.",
                            );
                        } else if release.package.is_none() {
                            ui.label("This release has no package for this kind of install.");
                        }
                        ui.horizontal(|ui| {
                            let can = install.can_update() && release.package.is_some();
                            let label = if matches!(install, Install::Downloads(_)) {
                                "Download now"
                            } else {
                                "Update now"
                            };
                            if can && ui.button(label).clicked() {
                                wanted = Some(release.clone());
                            }
                            if ui.button("Release page").clicked() {
                                open_in_browser(RELEASES_PAGE);
                            }
                        });
                    }
                    Phase::Working(step) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(step);
                        });
                    }
                    Phase::Done { message, restart: r } => {
                        ui.label(message);
                        if *r {
                            ui.weak("Save your project first: restarting closes it.");
                            if ui.button("Restart now").clicked() {
                                restart = true;
                            }
                        }
                    }
                    Phase::Failed(e) => {
                        ui.label(egui::RichText::new("The update did not work.").strong());
                        ui.label(e);
                        ui.horizontal(|ui| {
                            if ui.button("Try again").clicked() {
                                check_again = true;
                            }
                            if ui.button("Release page").clicked() {
                                open_in_browser(RELEASES_PAGE);
                            }
                        });
                    }
                }
            });
        if let Some(release) = wanted {
            self.update(ctx, release);
        }
        if check_again {
            *self.phase.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Idle;
            self.check(ctx);
        }
        if restart {
            relaunch(&install);
            // The program closes itself for the new version: no question about unsaved changes.
            crate::exit_prompt::allow_exit();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Closing the window while it works lets the work finish in the background.
        self.open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hotfix_comes_after_its_version() {
        assert!(is_newer("v0.2.0-hotfix", "0.2.0"));
        assert!(is_newer("v0.3.0", "0.2.0-hotfix"));
        assert!(is_newer("v0.2.1", "0.2.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("v0.2.0-hotfix", "0.2.0-hotfix"));
        assert!(!is_newer("v0.2.0", "0.2.0-hotfix"));
        assert!(!is_newer("v0.1.0-hotfix", "0.2.0"));
        assert!(is_newer("v0.2.0-hotfix2", "0.2.0-hotfix"));
    }

    fn answer() -> String {
        let asset = |name: &str| {
            format!(
                r#"{{"name":"{name}","browser_download_url":"{DOWNLOAD_PREFIX}v0.2.0-hotfix/{name}"}}"#
            )
        };
        format!(
            r#"{{"tag_name":"v0.2.0-hotfix","body":"New:\r\n- Preferences","assets":[{},{},{},{},{},{},{},{},{},{}]}}"#,
            asset("picasso-suite-0.2.0-hotfix-amd64.deb"),
            asset("picasso-suite-0.2.0-hotfix-arm64.deb"),
            asset("picasso-suite-0.2.0-hotfix-macos-universal.zip"),
            asset("picasso-suite-0.2.0-hotfix-windows.zip"),
            asset("picasso-suite-0.2.0-hotfix-x86_64.rpm"),
            asset("picasso-suite-0.2.0-hotfix-aarch64.rpm"),
            asset("picasso-suite-0.2.0-hotfix-x86_64.AppImage"),
            asset("picasso-suite-0.2.0-hotfix-aarch64.AppImage"),
            asset("picasso-suite-0.2.0-hotfix-aarch64.flatpak"),
            asset("picasso-suite-0.2.0-hotfix.flatpak"),
        )
    }

    #[test]
    fn each_install_gets_its_own_package() {
        let pick = |install: Install| {
            parse_release(&answer(), &install)
                .ok()
                .and_then(|r| r.package)
                .map(|(name, _)| name)
        };
        assert_eq!(
            pick(Install::WindowsZip(PathBuf::from("x"))).as_deref(),
            Some("picasso-suite-0.2.0-hotfix-windows.zip")
        );
        assert_eq!(
            pick(Install::MacApp(PathBuf::from("x"))).as_deref(),
            Some("picasso-suite-0.2.0-hotfix-macos-universal.zip")
        );
        let flatpak = if std::env::consts::ARCH == "aarch64" {
            "picasso-suite-0.2.0-hotfix-aarch64.flatpak"
        } else {
            "picasso-suite-0.2.0-hotfix.flatpak"
        };
        assert_eq!(pick(Install::Flatpak).as_deref(), Some(flatpak));
        // The Linux packages are this computer's kind of processor.
        let arch = std::env::consts::ARCH;
        let deb = if arch == "aarch64" { "arm64" } else { "amd64" };
        assert_eq!(pick(Install::Deb), Some(format!("picasso-suite-0.2.0-hotfix-{deb}.deb")));
        assert_eq!(pick(Install::Rpm), Some(format!("picasso-suite-0.2.0-hotfix-{arch}.rpm")));
        assert_eq!(
            pick(Install::AppImage(PathBuf::from("x"))),
            Some(format!("picasso-suite-0.2.0-hotfix-{arch}.AppImage"))
        );
        assert_eq!(pick(Install::Unmanaged), None);
    }

    #[test]
    fn a_copy_made_by_hand_gets_the_package_for_its_system() {
        let arch = std::env::consts::ARCH;
        let want = if cfg!(windows) {
            "picasso-suite-0.2.0-hotfix-windows.zip".to_string()
        } else if cfg!(target_os = "macos") {
            "picasso-suite-0.2.0-hotfix-macos-universal.zip".to_string()
        } else {
            format!("picasso-suite-0.2.0-hotfix-{arch}.AppImage")
        };
        let r = parse_release(&answer(), &Install::Downloads(PathBuf::from("x")));
        assert_eq!(r.map(|r| r.package.map(|(name, _)| name)), Ok(Some(want)));
        assert!(Install::Downloads(PathBuf::from("x")).can_update());
        assert!(!Install::Unmanaged.can_update());
    }

    #[test]
    fn the_answer_gives_the_tag_and_notes() {
        let r = parse_release(&answer(), &Install::Unmanaged);
        assert_eq!(r.as_ref().map(|r| r.tag.as_str()), Ok("v0.2.0-hotfix"));
        assert_eq!(r.map(|r| r.notes), Ok("New:\n- Preferences".to_string()));
    }

    #[test]
    fn files_from_elsewhere_are_not_downloaded() {
        let json = r#"{"tag_name":"v9.0.0","assets":[{"name":"picasso-suite-9.0.0-windows.zip","browser_download_url":"https://example.com/evil.zip"}]}"#;
        let r = parse_release(json, &Install::WindowsZip(PathBuf::from("x")));
        assert_eq!(r.map(|r| r.package), Ok(None));
        assert!(download("https://example.com/evil.zip", Path::new("x")).is_err());
    }

    #[test]
    fn a_per_user_flatpak_is_told_apart() {
        let user = "[Application]\nname=io.github.fabox_os.PicassoSuite\n\n[Instance]\n\
                    app-path=/home/me/.local/share/flatpak/app/io.github.fabox_os.PicassoSuite/x86_64/master/abc/files\n";
        let system = "[Instance]\n\
                      app-path=/var/lib/flatpak/app/io.github.fabox_os.PicassoSuite/x86_64/master/abc/files\n";
        assert!(is_user_flatpak(user));
        assert!(!is_user_flatpak(system));
        assert!(!is_user_flatpak(""));
    }

    #[test]
    fn each_package_goes_to_its_package_manager() {
        assert_eq!(package_manager(&Install::Deb), ("apt-get", &["install", "-y"][..]));
        // openSUSE (no dnf): the release's rpm is unsigned, which zypper refuses unless told.
        if !Path::new("/usr/bin/dnf").exists() {
            let (program, args) = package_manager(&Install::Rpm);
            assert_eq!(program, "zypper");
            assert!(args.contains(&"--allow-unsigned-rpm"), "{args:?}");
        }
    }

    #[test]
    fn a_copy_built_from_source_does_not_update_from_a_file() {
        // The test program is not an installed release, so nothing is touched: a package for
        // another system is refused.
        let r = update_from_file(Path::new("picasso-suite-9.9.9-windows.zip"));
        assert!(r.is_err());
        assert!(version_line().starts_with(&format!("picasso-suite {}", current_version())));
    }

    #[test]
    fn a_broken_answer_is_an_error() {
        assert!(parse_release("not json", &Install::Unmanaged).is_err());
        assert!(parse_release("{}", &Install::Unmanaged).is_err());
    }

    /// Asks the real GitHub (run with `--ignored`; needs the network and `curl`).
    #[test]
    #[ignore]
    fn the_latest_release_can_be_read() {
        let r = fetch_latest(&Install::WindowsZip(PathBuf::from("x")));
        let r = r.map_err(|e| e.to_string());
        assert!(r.as_ref().is_ok_and(|r| r.tag.starts_with('v')), "{r:?}");
        assert!(r.is_ok_and(|r| r.package.is_some()));
    }
}

#[cfg(all(test, windows))]
mod windows_install_tests {
    use super::*;

    /// Downloads the real latest Windows zip and installs it over a fake older copy in a temporary
    /// folder (run with `--ignored`; needs the network).
    #[test]
    #[ignore]
    fn the_windows_zip_installs_over_an_older_copy() {
        let dir = std::env::temp_dir().join("picasso-update-install-test");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(std::fs::create_dir_all(&dir).is_ok());
        for (name, text) in [("picasso-suite.exe", "old"), ("ffmpeg.exe", "old"), ("README.md", "old")] {
            assert!(std::fs::write(dir.join(name), text).is_ok());
        }
        let install = Install::WindowsZip(dir.clone());
        let release = fetch_latest(&install);
        assert!(release.is_ok(), "{release:?}");
        let Ok(release) = release else { return };
        let done = download_and_install(&install, &release, &|_| {});
        assert!(done.as_ref().is_ok_and(|d| d.restart), "{:?}", done.err());
        let size = |name: &str| std::fs::metadata(dir.join(name)).map(|m| m.len()).unwrap_or(0);
        assert!(size("picasso-suite.exe") > 1_000_000);
        assert!(size("ffmpeg.exe") > 1_000_000);
        assert_eq!(std::fs::read_to_string(dir.join("picasso-suite.exe.old")).ok().as_deref(), Some("old"));
        clean_up_dir(&dir);
        assert!(!dir.join("picasso-suite.exe.old").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    #[test]
    fn each_update_gets_a_folder_of_its_own_that_nobody_else_made() {
        let one = work_dir("v9.9.9").unwrap();
        let two = work_dir("v9.9.9").unwrap();
        assert_ne!(one, two, "a name nobody can guess, never one that was there before");
        assert!(two.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&two).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(&two);
    }

    #[test]
    fn a_package_name_that_is_a_path_is_not_taken() {
        let json = r#"{"tag_name":"v9.9.9","body":"","assets":[
            {"name":"../../picasso-suite-windows.zip","browser_download_url":"https://github.com/fabox-os/Picasso-Suite/releases/download/v9.9.9/evil.zip"},
            {"name":".picasso-suite-windows.zip","browser_download_url":"https://github.com/fabox-os/Picasso-Suite/releases/download/v9.9.9/h.zip"}]}"#;
        let release = parse_release(json, &Install::WindowsZip(PathBuf::from("."))).unwrap_or_else(|_| panic!("parsed"));
        assert!(release.package.is_none(), "{:?}", release.package);
    }
}
