//! What is known about the media of a project (size, frame rate, codecs, length), and the
//! project file that keeps it: a SQLite database holding the project as JSON next to a table of
//! media facts (Roadmap: "Project file .picasso (json + sqlite)"). Desktop only; the browser
//! build keeps projects as plain JSON text.

use std::io::Cursor;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, MAIN_DB};
use thiserror::Error;

/// The application id written in the database header ("PICS").
const APPLICATION_ID: i64 = 0x5049_4353;
/// The version of the file layout this program writes and reads.
const FORMAT_VERSION: i64 = 1;
/// Databases bigger than this are refused.
const MAX_BYTES: usize = 256 * 1024 * 1024;
/// The first bytes of every SQLite database.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// A database that takes longer than this to read is refused: the file may come from anyone, and
/// a view or a recursive query in it can run for ever. A real project is read in milliseconds.
const READ_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("the file is not a Picasso Suite project database: {0}")]
    NotAProject(String),
    #[error("the project file is too big")]
    TooBig,
    #[error("the project file takes too long to read")]
    TooSlow,
    #[error("the project file is damaged ({0})")]
    Corrupt(String),
    #[error("the project file was made by a newer version (file layout {0})")]
    TooNew(i64),
    #[error("database error: {0}")]
    Database(String),
    #[error("could not read the media file: {0}")]
    Probe(String),
}

impl From<rusqlite::Error> for MediaError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

/// What is known about one media item of a project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaMetadata {
    /// The id of the media item in the project.
    pub media: u64,
    /// "video", "audio" or "image".
    pub kind: String,
    pub path: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Frames and seconds, e.g. (30000, 1001).
    pub fps: Option<(u32, u32)>,
    pub duration_us: Option<i64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub file_size: Option<u64>,
    /// The start timecode the file carries ("01:00:00:00"), when it has one.
    pub timecode: Option<String>,
}

fn size_of_file(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

impl MediaMetadata {
    /// Reads the facts of a video file.
    pub fn probe_video(media: u64, path: &Path) -> Result<Self, MediaError> {
        let info = picasso_decode::probe(path).map_err(|e| MediaError::Probe(e.to_string()))?;
        Ok(Self {
            media,
            kind: "video".into(),
            path: path.to_string_lossy().into_owned(),
            width: Some(info.width),
            height: Some(info.height),
            fps: Some(info.fps),
            duration_us: info.duration_us,
            video_codec: Some(info.video_codec),
            audio_codec: info.audio_codec,
            file_size: size_of_file(path),
            timecode: info.timecode,
        })
    }

    /// Reads the facts of a sound file.
    pub fn probe_audio(media: u64, path: &Path) -> Result<Self, MediaError> {
        let duration_us = picasso_decode::audio::probe_audio(path)
            .map_err(|e| MediaError::Probe(e.to_string()))?;
        Ok(Self {
            media,
            kind: "audio".into(),
            path: path.to_string_lossy().into_owned(),
            duration_us,
            file_size: size_of_file(path),
            ..Self::default()
        })
    }

    /// The facts of a still image that has been read already.
    pub fn image(media: u64, path: &str, width: u32, height: u32) -> Self {
        Self {
            media,
            kind: "image".into(),
            path: path.to_string(),
            width: Some(width),
            height: Some(height),
            file_size: size_of_file(Path::new(path)),
            ..Self::default()
        }
    }

    /// One line for a tooltip: "1920 x 1080, 24 fps, h264 + aac, 4.0 s".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let (Some(w), Some(h)) = (self.width, self.height) {
            parts.push(format!("{w} x {h}"));
        }
        if let Some((n, d)) = self.fps.filter(|(n, d)| *n > 0 && *d > 0) {
            let fps = f64::from(n) / f64::from(d);
            parts.push(if (fps - fps.round()).abs() < 0.005 {
                format!("{fps:.0} fps")
            } else {
                format!("{fps:.2} fps")
            });
        }
        if let Some(tc) = &self.timecode {
            parts.push(format!("TC {tc}"));
        }
        match (&self.video_codec, &self.audio_codec) {
            (Some(v), Some(a)) => parts.push(format!("{v} + {a}")),
            (Some(v), None) => parts.push(v.clone()),
            (None, Some(a)) => parts.push(a.clone()),
            (None, None) => {}
        }
        if let Some(us) = self.duration_us {
            parts.push(format!("{:.1} s", us as f64 / 1_000_000.0));
        }
        if let Some(bytes) = self.file_size {
            parts.push(format!("{:.1} MB", bytes as f64 / 1_048_576.0));
        }
        parts.join(", ")
    }
}

/// Whether `bytes` start like a SQLite database (and so are a project database, not JSON).
pub fn is_database(bytes: &[u8]) -> bool {
    bytes.starts_with(SQLITE_MAGIC)
}

/// A project database as bytes: the project's JSON text and the facts about its media.
pub fn to_bytes(project_json: &str, media: &[MediaMetadata]) -> Result<Vec<u8>, MediaError> {
    let mut db = Connection::open_in_memory()?;
    db.pragma_update(None, "application_id", APPLICATION_ID)?;
    db.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE project (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);
         CREATE TABLE media (
             id INTEGER PRIMARY KEY,
             kind TEXT NOT NULL,
             path TEXT NOT NULL,
             width INTEGER, height INTEGER,
             fps_num INTEGER, fps_den INTEGER,
             duration_us INTEGER,
             video_codec TEXT, audio_codec TEXT,
             file_size INTEGER,
             timecode TEXT
         );",
    )?;
    let tx = db.transaction()?;
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('format', 'picasso-project'), ('version', ?1)",
        params![FORMAT_VERSION.to_string()],
    )?;
    tx.execute(
        "INSERT INTO project (id, json) VALUES (1, ?1)",
        params![project_json],
    )?;
    for m in media {
        tx.execute(
            "INSERT OR REPLACE INTO media VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                i64::try_from(m.media).unwrap_or(i64::MAX),
                m.kind,
                m.path,
                m.width,
                m.height,
                m.fps.map(|f| f.0),
                m.fps.map(|f| f.1),
                m.duration_us,
                m.video_codec,
                m.audio_codec,
                m.file_size.and_then(|s| i64::try_from(s).ok()),
                m.timecode,
            ],
        )?;
    }
    tx.commit()?;
    let data = db.serialize(MAIN_DB)?;
    Ok(data.to_vec())
}

/// Reads a project database: the project's JSON text and the facts about its media.
pub fn from_bytes(bytes: &[u8]) -> Result<(String, Vec<MediaMetadata>), MediaError> {
    from_bytes_within(bytes, READ_BUDGET)
}

/// `from_bytes` with the time it may take: past it, reading stops and the file is refused.
fn from_bytes_within(
    bytes: &[u8],
    budget: std::time::Duration,
) -> Result<(String, Vec<MediaMetadata>), MediaError> {
    if bytes.len() > MAX_BYTES {
        return Err(MediaError::TooBig);
    }
    if !is_database(bytes) {
        return Err(MediaError::NotAProject("it is not a database".into()));
    }
    let mut db = Connection::open_in_memory()?;
    db.deserialize_read_exact(MAIN_DB, Cursor::new(bytes), bytes.len(), true)
        .map_err(|e| MediaError::Corrupt(e.to_string()))?;
    // The file may come from anyone: its views and triggers must not run unsafe functions,
    // reading it must not change anything, and it must not run for ever: SQLite asks after every
    // thousand steps and stops the query once the time is up.
    db.execute_batch("PRAGMA trusted_schema = OFF; PRAGMA query_only = ON;")?;
    let deadline = std::time::Instant::now() + budget;
    db.progress_handler(1000, Some(move || std::time::Instant::now() > deadline))?;
    // SQLite's own check of every page before any table is trusted: a file cut short, or one
    // whose pages were damaged under it, can still deserialize and answer the first query, and
    // would otherwise open as a project that lost half its media. The progress handler above
    // stops a check that would take for ever.
    let checked = quick_check(&db);
    if checked.is_err() && std::time::Instant::now() > deadline {
        return Err(MediaError::TooSlow);
    }
    let problems = checked.map_err(MediaError::Corrupt)?;
    if problems.len() != 1 || problems[0] != "ok" {
        return Err(MediaError::Corrupt(problems.join("; ")));
    }
    let read = read_project(&db);
    // Whatever the error said, running out of time is the reason.
    if read.is_err() && std::time::Instant::now() > deadline {
        return Err(MediaError::TooSlow);
    }
    read
}

/// `PRAGMA quick_check` on an opened database: `["ok"]` when every page and index is sound,
/// and otherwise SQLite's own description of what is wrong.
fn quick_check(db: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = db
        .prepare("PRAGMA quick_check")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The project and the media facts in an opened project database.
fn read_project(db: &Connection) -> Result<(String, Vec<MediaMetadata>), MediaError> {
    let not_project = |what: &str| MediaError::NotAProject(what.to_string());
    let format: Option<String> = db
        .query_row("SELECT value FROM meta WHERE key = 'format'", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|_| not_project("it has no project tables"))?;
    if format.as_deref() != Some("picasso-project") {
        return Err(not_project("it is a different kind of database"));
    }
    let version: i64 = db
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| not_project("it has no version"))?
        .parse()
        .map_err(|_| not_project("its version is not a number"))?;
    if version > FORMAT_VERSION {
        return Err(MediaError::TooNew(version));
    }
    let json: String = db
        .query_row("SELECT json FROM project WHERE id = 1", [], |r| r.get(0))
        .map_err(|_| not_project("it holds no project"))?;
    // Databases written before the timecode column existed have no `timecode`; they read as
    // having none, so an older project still opens.
    let has_timecode = {
        let mut probe = db.prepare("PRAGMA table_info(media)")?;
        let mut found = false;
        for name in probe.query_map([], |r| r.get::<_, String>(1))?.flatten() {
            if name == "timecode" {
                found = true;
                break;
            }
        }
        found
    };
    let sql = if has_timecode {
        "SELECT id, kind, path, width, height, fps_num, fps_den, duration_us,
                video_codec, audio_codec, file_size, timecode FROM media ORDER BY id
         LIMIT 100000" // no real project has more media than that
    } else {
        "SELECT id, kind, path, width, height, fps_num, fps_den, duration_us,
                video_codec, audio_codec, file_size, NULL FROM media ORDER BY id
         LIMIT 100000"
    };
    let mut stmt = db.prepare(sql)?;
    let media = stmt
        .query_map([], |r| {
            let (n, d): (Option<u32>, Option<u32>) = (r.get(5)?, r.get(6)?);
            Ok(MediaMetadata {
                media: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                kind: r.get(1)?,
                path: r.get(2)?,
                width: r.get(3)?,
                height: r.get(4)?,
                fps: n.zip(d),
                duration_us: r.get(7)?,
                video_codec: r.get(8)?,
                audio_codec: r.get(9)?,
                file_size: r
                    .get::<_, Option<i64>>(10)?
                    .and_then(|s| u64::try_from(s).ok()),
                timecode: r.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((json, media))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<MediaMetadata> {
        vec![
            MediaMetadata {
                media: 3,
                kind: "video".into(),
                path: "C:/clips/a.mp4".into(),
                width: Some(1920),
                height: Some(1080),
                fps: Some((30000, 1001)),
                duration_us: Some(4_000_000),
                video_codec: Some("h264".into()),
                audio_codec: Some("aac".into()),
                file_size: Some(5_242_880),
                timecode: Some("01:00:00:00".into()),
            },
            MediaMetadata::image(7, "no-such-file.png", 64, 48),
        ]
    }

    #[test]
    fn a_project_survives_the_database() {
        let json = r#"{"name":"x","tracks":[]}"#;
        let bytes = to_bytes(json, &sample()).unwrap();
        assert!(is_database(&bytes));
        let (back, media) = from_bytes(&bytes).unwrap();
        assert_eq!(back, json);
        assert_eq!(media, sample());
    }

    #[test]
    fn json_text_and_other_files_are_not_project_databases() {
        assert!(!is_database(b"{\"name\": \"x\"}"));
        assert!(matches!(
            from_bytes(b"{\"name\": \"x\"}"),
            Err(MediaError::NotAProject(_))
        ));
        assert!(from_bytes(b"").is_err());
        // A real SQLite database of something else.
        let other = {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch("CREATE TABLE t (x)").unwrap();
            db.serialize(MAIN_DB).unwrap().to_vec()
        };
        assert!(matches!(
            from_bytes(&other),
            Err(MediaError::NotAProject(_))
        ));
        // A project database cut short or with garbage in it is refused, not trusted.
        let mut bytes = to_bytes("{}", &[]).unwrap();
        bytes.truncate(bytes.len() / 2);
        assert!(from_bytes(&bytes).is_err());
        let mut junk = to_bytes("{}", &[]).unwrap();
        for b in junk.iter_mut().skip(100).take(300) {
            *b = 0xA5;
        }
        assert!(from_bytes(&junk).is_err());
    }

    #[test]
    fn a_database_with_a_lost_page_is_damaged() {
        // A page appended past the last one and counted in the header: every table still reads,
        // and only SQLite's own check says the file is wrong ("Page 6: never used"). A project
        // file that lost a page must not open as if nothing had happened.
        let mut bytes = to_bytes("{}", &sample()).unwrap();
        let pages = bytes.len() / 4096;
        bytes.resize(bytes.len() + 4096, 0);
        bytes[28..32].copy_from_slice(&(u32::try_from(pages + 1).unwrap()).to_be_bytes());
        let read = from_bytes(&bytes);
        assert!(read.is_err(), "a database with a lost page was opened");
        let message = read.unwrap_err().to_string();
        assert!(
            message.starts_with("the project file is damaged"),
            "{message}"
        );
    }

    #[test]
    fn a_database_that_never_finishes_is_refused_in_time() {
        // A `media` view that counts for ever and sorts what it counts: the file looks right but
        // reading it would hang the program.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES ('format', 'picasso-project'), ('version', '1');
             CREATE TABLE project (id INTEGER PRIMARY KEY, json TEXT NOT NULL);
             INSERT INTO project VALUES (1, '{}');
             CREATE VIEW media AS
               WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c)
               SELECT x AS id, 'video' AS kind, 'p' AS path, NULL AS width, NULL AS height,
                      NULL AS fps_num, NULL AS fps_den, NULL AS duration_us,
                      NULL AS video_codec, NULL AS audio_codec, NULL AS file_size FROM c;",
        )
        .unwrap();
        let bytes = db.serialize(MAIN_DB).unwrap().to_vec();
        drop(db);
        let started = std::time::Instant::now();
        let read = from_bytes_within(&bytes, std::time::Duration::from_millis(300));
        assert!(matches!(read, Err(MediaError::TooSlow)), "{read:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "it stopped late: {:?}",
            started.elapsed()
        );
        // A real project is far inside the budget.
        let bytes = to_bytes("{}", &sample()).unwrap();
        assert!(from_bytes_within(&bytes, std::time::Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn a_newer_layout_is_refused() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES ('format', 'picasso-project'), ('version', '99');",
        )
        .unwrap();
        let bytes = db.serialize(MAIN_DB).unwrap().to_vec();
        drop(db);
        assert!(matches!(from_bytes(&bytes), Err(MediaError::TooNew(99))));
    }

    #[test]
    fn a_database_without_a_timecode_column_still_opens() {
        // The layout written before timecodes were kept: the same tables without `timecode`.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES ('format', 'picasso-project'), ('version', '1');
             CREATE TABLE project (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);
             INSERT INTO project VALUES (1, '{\"name\":\"old\"}');
             CREATE TABLE media (
                 id INTEGER PRIMARY KEY, kind TEXT NOT NULL, path TEXT NOT NULL,
                 width INTEGER, height INTEGER, fps_num INTEGER, fps_den INTEGER,
                 duration_us INTEGER, video_codec TEXT, audio_codec TEXT, file_size INTEGER
             );
             INSERT INTO media VALUES (1, 'video', 'a.mp4', 1920, 1080, 24, 1,
                                      2000000, 'h264', 'aac', 1000);",
        )
        .unwrap();
        let bytes = db.serialize(MAIN_DB).unwrap().to_vec();
        drop(db);
        let (json, media) = from_bytes(&bytes).unwrap();
        assert_eq!(json, r#"{"name":"old"}"#);
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].width, Some(1920));
        assert_eq!(media[0].timecode, None);
    }

    #[test]
    fn the_summary_reads_like_a_tooltip() {
        assert_eq!(
            sample()[0].summary(),
            "1920 x 1080, 29.97 fps, TC 01:00:00:00, h264 + aac, 4.0 s, 5.0 MB"
        );
        assert_eq!(sample()[1].summary(), "64 x 48");
        assert_eq!(MediaMetadata::default().summary(), "");
    }

    /// A two-second clip with a tone, made with the `ffmpeg` program.
    fn clip() -> Option<std::path::PathBuf> {
        let path = std::env::temp_dir().join("picasso_media_test.mp4");
        std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=duration=2:size=320x180:rate=24")
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args(["-pix_fmt", "yuv420p", "-shortest"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success())
            .then_some(path)
    }

    #[test]
    fn real_files_are_probed() {
        let Some(path) = clip() else { return };
        let video = MediaMetadata::probe_video(5, &path).unwrap();
        assert_eq!(
            (video.width, video.height, video.fps),
            (Some(320), Some(180), Some((24, 1)))
        );
        assert_eq!(video.video_codec.as_deref(), Some("h264"));
        assert_eq!(video.audio_codec.as_deref(), Some("aac"));
        assert!(video.file_size.is_some_and(|s| s > 1000));
        assert!(video
            .duration_us
            .is_some_and(|d| (1_900_000..2_200_000).contains(&d)));
        let audio = MediaMetadata::probe_audio(6, &path).unwrap();
        assert_eq!(audio.kind, "audio");
        assert!(matches!(
            MediaMetadata::probe_video(1, Path::new("missing.mp4")),
            Err(MediaError::Probe(_))
        ));
        let _ = std::fs::remove_file(path);
    }
}
