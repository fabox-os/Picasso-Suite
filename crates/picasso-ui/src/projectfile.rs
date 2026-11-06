//! Project files. On the desktop a project is a SQLite database (the project as JSON next to a
//! table of facts about its media); the browser keeps plain JSON text. Reading accepts both, so
//! older projects, and projects saved in the browser, still open.

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use picasso_media::MediaMetadata;

/// What is known about a media item; the browser knows nothing beyond the picture itself.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MediaMetadata {
    pub media: u64,
}

#[cfg(target_arch = "wasm32")]
impl MediaMetadata {
    pub(crate) fn summary(&self) -> String {
        String::new()
    }
}

/// The bytes of a project file: the project's JSON and the facts about its media.
pub(crate) fn encode(json: &str, facts: &[MediaMetadata]) -> Result<Vec<u8>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        picasso_media::to_bytes(json, facts).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = facts;
        Ok(json.as_bytes().to_vec())
    }
}

/// The project's JSON and the facts about its media from the bytes of a project file, which is a
/// database or JSON text.
pub(crate) fn decode(bytes: Vec<u8>) -> Result<(String, Vec<MediaMetadata>), String> {
    #[cfg(not(target_arch = "wasm32"))]
    if picasso_media::is_database(&bytes) {
        return picasso_media::from_bytes(&bytes).map_err(|e| e.to_string());
    }
    String::from_utf8(bytes)
        .map(|text| (text, Vec::new()))
        .map_err(|_| "that file is not a project (it is neither a database nor text)".to_string())
}

/// The project in a file the user opened, and the way back when the file is damaged: a
/// database that fails SQLite's own check is reported with the newest autosave beside it, which
/// is what to open instead. Files that are not damaged are read exactly as `decode` reads them.
pub(crate) fn decode_open(
    bytes: Vec<u8>,
    project: Option<&std::path::Path>,
) -> Result<(String, Vec<MediaMetadata>), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if picasso_media::is_database(&bytes) {
            return picasso_media::from_bytes(&bytes).map_err(|e| damaged(e, project));
        }
    }
    let _ = project;
    decode(bytes)
}

/// What to tell someone whose project file is damaged: SQLite's own reason, and the newest
/// autosave copy to recover from (the project's, kept beside it, or none at all).
#[cfg(not(target_arch = "wasm32"))]
fn damaged(e: picasso_media::MediaError, project: Option<&std::path::Path>) -> String {
    if !matches!(e, picasso_media::MediaError::Corrupt(_)) {
        return e.to_string();
    }
    match project.and_then(|path| crate::autosave::newest(Some(path), "")) {
        Some(copy) => format!(
            "{e}; the newest autosave copy to recover from is {}",
            copy.display()
        ),
        None => format!("{e}; there is no autosave copy to recover from"),
    }
}

/// The facts of a still image.
pub(crate) fn image_facts(
    media: u64,
    path: &str,
    width: u32,
    height: u32,
) -> Option<MediaMetadata> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(MediaMetadata::image(media, path, width, height))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (media, path, width, height);
        None
    }
}
