#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! Reads a real speech sample with a real model. Ignored by default (the model is 150 MB);
//! run with `PICASSO_ASR_TEST_DIR` set to a folder that holds `whisper-tiny/` (the model, as
//! `picasso_asr::models` keeps it) and `jfk.f32` (the sample as raw 16 kHz mono f32 samples):
//!
//! `PICASSO_ASR_TEST_DIR=... cargo test -p picasso-asr --release -- --ignored --nocapture`

use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
    std::env::var_os("PICASSO_ASR_TEST_DIR").map(PathBuf::from)
}

fn sample(dir: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(dir.join("jfk.f32")).expect("the sample");
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

#[test]
#[ignore]
fn a_known_speech_sample_is_read_as_what_was_said() {
    let Some(dir) = dir() else {
        eprintln!("PICASSO_ASR_TEST_DIR is not set");
        return;
    };
    let pcm = sample(&dir);
    let started = std::time::Instant::now();
    // `PICASSO_ASR_TEST_MODEL` names the folder of the model (default: whisper-tiny).
    let model = std::env::var("PICASSO_ASR_TEST_MODEL").unwrap_or_else(|_| "whisper-tiny".into());
    let mut whisper = picasso_asr::Whisper::load(&dir.join(&model)).expect("the model loads");
    eprintln!("loaded in {:?}", started.elapsed());
    assert_eq!(whisper.languages().len() > 50, whisper.multilingual());
    let mut steps = 0;
    let transcript = whisper
        .transcribe(&pcm, None, &mut |share, what| {
            steps += 1;
            eprintln!("{share:.2} {what}");
            true
        })
        .expect("the sound is read");
    eprintln!("read in {:?}: {transcript:#?}", started.elapsed());
    if whisper.multilingual() {
        assert_eq!(transcript.language.as_deref(), Some("en"));
    } else {
        assert_eq!(transcript.language, None);
    }
    let all: String = transcript
        .segments
        .iter()
        .map(|s| s.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    for word in ["fellow americans", "ask not", "country can do for you", "your country"] {
        assert!(all.contains(word), "\"{word}\" is not in \"{all}\"");
    }
    // The times are in order and inside the sound (about eleven seconds).
    let mut last = 0.0;
    for s in &transcript.segments {
        assert!(s.start >= last - 1e-6 && s.end >= s.start && s.end <= 12.5, "{s:?}");
        last = s.start;
    }
    // The end of the quote is not made into "Thank you.": the last thing said runs to the end.
    assert!(!all.contains("thank you"), "{all}");
    assert!((transcript.segments.last().map_or(0.0, |s| s.end) - 11.0).abs() < 0.2);
    let cues = picasso_asr::cues::pack(&transcript.segments, 7, 5.0);
    assert!(cues.len() >= 2, "{cues:?}");
    assert!(steps >= 2);
    // Stopping on request.
    let stopped = whisper.transcribe(&pcm, None, &mut |_, _| false);
    assert!(stopped.is_err());
}

/// Needs the internet: downloads Whisper tiny (English), checks it, and loads it.
#[test]
#[ignore]
fn a_model_is_downloaded_checked_installed_and_loads() {
    let base = std::env::temp_dir().join("picasso_asr_download_test");
    let _ = std::fs::remove_dir_all(&base);
    let model = picasso_asr::models::by_id("whisper-tiny.en").expect("in the list");
    assert!(!picasso_asr::models::is_installed(&base, model));
    // Stopping at once leaves nothing under the files' names.
    let stopped = picasso_asr::models::download(&base, model, &mut |_, _| false);
    assert!(stopped.is_err());
    assert!(!picasso_asr::models::is_installed(&base, model));
    assert!(!picasso_asr::models::dir_of(&base, model).join("model.safetensors").exists());
    // Downloading it all: progress goes up to the whole size.
    let mut last = 0;
    picasso_asr::models::download(&base, model, &mut |done, total| {
        assert!(done <= total);
        last = done;
        true
    })
    .expect("downloads");
    assert_eq!(last, model.bytes());
    assert!(picasso_asr::models::is_installed(&base, model));
    // The second time there is nothing to fetch, and it loads: an English-only model.
    picasso_asr::models::download(&base, model, &mut |_, _| true).expect("already there");
    let whisper = picasso_asr::Whisper::load(&picasso_asr::models::dir_of(&base, model)).expect("loads");
    assert!(whisper.languages().is_empty());
    // A damaged file is found and not kept: the same size, other contents.
    let weights = picasso_asr::models::dir_of(&base, model).join("model.safetensors");
    let mut bytes = std::fs::read(&weights).unwrap();
    bytes[1000] ^= 0xff;
    std::fs::write(&weights, &bytes).unwrap();
    assert!(picasso_asr::models::sha256_of(&weights).unwrap() != model.files[2].sha256);
    let _ = std::fs::remove_dir_all(&base);
}
