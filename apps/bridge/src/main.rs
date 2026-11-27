//! `picasso-bridge`: moves DaVinci Resolve projects into Picasso Suite from the command line.
//!
//! ```text
//! picasso-bridge info <file.drp>              what the file holds, and which media is missing
//! picasso-bridge convert <file.drp> [out]     writes a Picasso Suite project (default: next to it)
//! picasso-bridge export-otio <file.drp> [out.otio]
//!                                           writes an OpenTimelineIO file (default: next to it)
//! picasso-bridge install-linux [--exec PATH]  opens Resolve files with Picasso Suite from the
//!                                             desktop (Linux: a .desktop entry and MIME types)
//! ```
//!
//! It reads `.drp` (project), `.drt` (timeline) and `.drb` (bin) archives and plain XML exports;
//! see the `picasso-drp` crate for what is read from them.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  picasso-bridge info <file.drp>
  picasso-bridge convert <file.drp> [out.picasso]
  picasso-bridge export-otio <file.drp> [out.otio]
  picasso-bridge install-linux [--exec /path/to/picasso-suite]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") => args.get(1).ok_or(USAGE.to_string()).and_then(|f| info(f)),
        Some("convert") => args
            .get(1)
            .ok_or(USAGE.to_string())
            .and_then(|f| convert(f, args.get(2).map(String::as_str))),
        Some("export-otio") => args
            .get(1)
            .ok_or(USAGE.to_string())
            .and_then(|f| export_otio(f, args.get(2).map(String::as_str))),
        Some("install-linux") => {
            let exec = args
                .iter()
                .position(|a| a == "--exec")
                .and_then(|i| args.get(i + 1))
                .cloned();
            install_linux(exec)
        }
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn read(file: &str) -> Result<picasso_drp::DrpProject, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    picasso_drp::read(&bytes).map_err(|e| format!("{file}: {e}"))
}

fn stem(file: &str) -> String {
    Path::new(file).file_stem().map_or_else(
        || "Imported Project".to_string(),
        |s| s.to_string_lossy().into_owned(),
    )
}

fn info(file: &str) -> Result<(), String> {
    let p = read(file)?;
    println!(
        "Project:     {}",
        p.name.clone().unwrap_or_else(|| stem(file))
    );
    println!(
        "Frame rate:  {}",
        p.fps.map_or("not given".to_string(), |f| format!("{f}"))
    );
    match (p.width, p.height) {
        (Some(w), Some(h)) => println!("Picture:     {w} x {h}"),
        _ => println!("Picture:     not given"),
    }
    println!(
        "Documents:   {} of {} entries",
        p.documents,
        p.entries.len()
    );
    let missing = p
        .media
        .iter()
        .filter(|m| !Path::new(&m.path).exists())
        .count();
    println!("Media:       {} ({missing} not found here)", p.media.len());
    for m in &p.media {
        let here = if Path::new(&m.path).exists() {
            "  "
        } else {
            "? "
        };
        println!("  {here}{:?}  {}", m.kind, m.path);
    }
    Ok(())
}

fn convert(file: &str, out: Option<&str>) -> Result<(), String> {
    let p = read(file)?;
    let project = picasso_drp::to_project(&p, &stem(file));
    let text = project.to_json().map_err(|e| e.to_string())?;
    let out = out.map_or_else(|| Path::new(file).with_extension("picasso"), PathBuf::from);
    std::fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "Wrote {} ({} media files)",
        out.display(),
        project.media.len()
    );
    Ok(())
}

fn export_otio(file: &str, out: Option<&str>) -> Result<(), String> {
    let p = read(file)?;
    let project = picasso_drp::to_project(&p, &stem(file));
    let otio = picasso_core::otio::to_otio(&project);
    let out = out.map_or_else(
        || Path::new(file).with_extension("otio"),
        PathBuf::from,
    );
    std::fs::write(&out, otio).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("Wrote {}", out.display());
    Ok(())
}

/// The MIME types of Resolve's files, for the desktop's file associations.
const MIME_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-davinci-resolve-project">
    <comment>DaVinci Resolve project</comment>
    <glob pattern="*.drp"/>
  </mime-type>
  <mime-type type="application/x-davinci-resolve-timeline">
    <comment>DaVinci Resolve timeline</comment>
    <glob pattern="*.drt"/>
  </mime-type>
  <mime-type type="application/x-davinci-resolve-bin">
    <comment>DaVinci Resolve bin</comment>
    <glob pattern="*.drb"/>
  </mime-type>
  <mime-type type="application/x-picasso-project">
    <comment>Picasso Suite project</comment>
    <glob pattern="*.picasso"/>
  </mime-type>
</mime-info>
"#;

fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]
Type=Application
Name=Picasso Suite
Comment=Video editor; opens DaVinci Resolve projects
Exec={exec} %f
Icon=picasso-suite
Terminal=false
Categories=AudioVideo;Video;AudioVideoEditing;
MimeType=application/x-picasso-project;application/x-davinci-resolve-project;application/x-davinci-resolve-timeline;application/x-davinci-resolve-bin;
"
    )
}

fn install_linux(exec: Option<String>) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err(
            "install-linux registers Resolve files with Linux desktops; on this system open them \
             with File > Open in Picasso Suite"
                .to_string(),
        );
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let data = std::env::var_os("XDG_DATA_HOME")
        .map_or_else(|| PathBuf::from(&home).join(".local/share"), PathBuf::from);
    // The app sits next to this tool unless told otherwise.
    let exec = exec.unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("picasso-suite")))
            .filter(|p| p.exists())
            .map_or_else(|| "picasso-suite".to_string(), |p| p.display().to_string())
    });
    let apps = data.join("applications");
    let mime = data.join("mime/packages");
    for dir in [&apps, &mime] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let desktop = apps.join("picasso-suite.desktop");
    std::fs::write(&desktop, desktop_entry(&exec))
        .map_err(|e| format!("{}: {e}", desktop.display()))?;
    let mime_file = mime.join("picasso-suite.xml");
    std::fs::write(&mime_file, MIME_XML).map_err(|e| format!("{}: {e}", mime_file.display()))?;
    // Let the desktop see the new types; these tools are optional.
    let _ = std::process::Command::new("update-mime-database")
        .arg(data.join("mime"))
        .status();
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps)
        .status();
    println!("Registered Picasso Suite for .drp, .drt, .drb and .picasso files");
    println!("  {}", desktop.display());
    println!("  {}", mime_file.display());
    println!("Starts: {exec}");
    Ok(())
}
