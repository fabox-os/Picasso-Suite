//! The Whisper models: which there are, where they are kept and how they are downloaded. A model
//! is three files from OpenAI's repository on Hugging Face (its settings, its words and its
//! weights), pinned to one revision and checked against their SHA-256 when they arrive, so what is
//! run is what was meant. They are downloaded when first wanted, with the system's `curl` (the
//! program carries no network code of its own), into a folder of the user's.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{AsrError, Result};

/// Files are only downloaded from here.
const PREFIX: &str = "https://huggingface.co/openai/";

/// One file of a model.
#[derive(Clone, Copy, Debug)]
pub struct ModelFile {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// A Whisper model.
#[derive(Clone, Copy, Debug)]
pub struct Model {
    /// The name of its folder and of the repository (`whisper-tiny`).
    pub id: &'static str,
    pub label: &'static str,
    /// What it is good for, in a few words.
    pub note: &'static str,
    /// The commit of the repository the files are taken from.
    pub revision: &'static str,
    /// Whether it knows languages other than English.
    pub multilingual: bool,
    pub files: [ModelFile; 3],
}

impl Model {
    /// The size of all the files, in bytes.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    /// Where `file` comes from.
    pub fn url(&self, file: &ModelFile) -> String {
        format!("{PREFIX}{}/resolve/{}/{}", self.id, self.revision, file.name)
    }
}

const fn file(name: &'static str, bytes: u64, sha256: &'static str) -> ModelFile {
    ModelFile { name, bytes, sha256 }
}

const TOKENS_EN: &str = "5eb60cec1e77aeeb6869a2bb5a8e01a84c3fe5d072d75369343021fe6f5310d0";
const TOKENS_ALL: &str = "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566";

/// The models, from the smallest and quickest to the largest and most careful.
pub const MODELS: [Model; 6] = [
    Model {
        id: "whisper-tiny.en",
        label: "Tiny (English)",
        note: "Fastest, English only",
        revision: "87c7102498dcde7456f24cfd30239ca606ed9063",
        multilingual: false,
        files: [
            file("config.json", 1937, "b5366317df280f79e6cc366f259f379674f81a8721d458acf2e00680ccb10b1d"),
            file("tokenizer.json", 2_405_679, TOKENS_EN),
            file("model.safetensors", 151_060_136, "db59695928ded6043adaef491a53ef4e12da9611184d77c53baa691a60b958ad"),
        ],
    },
    Model {
        id: "whisper-tiny",
        label: "Tiny",
        note: "Fastest, many languages",
        revision: "169d4a4341b33bc18d8881c4b69c2e104e1cc0af",
        multilingual: true,
        files: [
            file("config.json", 1983, "ffdccec4f3211f4c63310f2b7098f309fe70f3952cedc5e4d11e43f5b2379b98"),
            file("tokenizer.json", 2_480_466, TOKENS_ALL),
            file("model.safetensors", 151_061_672, "7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395"),
        ],
    },
    Model {
        id: "whisper-base.en",
        label: "Base (English)",
        note: "Quick and better, English only",
        revision: "911407f4214e0e1d82085af863093ec0b66f9cd6",
        multilingual: false,
        files: [
            file("config.json", 1937, "160c1df40a60d4ef5a6014d536fd845c33aabfd1f318458ac7f87cfe91ebbe76"),
            file("tokenizer.json", 2_405_679, TOKENS_EN),
            file("model.safetensors", 290_401_888, "d4dd5542fd6a1d35639e21384238f3bfe6c557c849d392b5905d33ee29e71db5"),
        ],
    },
    Model {
        id: "whisper-base",
        label: "Base",
        note: "Quick and better, many languages",
        revision: "e37978b90ca9030d5170a5c07aadb050351a65bb",
        multilingual: true,
        files: [
            file("config.json", 1983, "a153c53883a6799b6f056b4a8d1a515c9926d03994682ba88a7616618d7da0c1"),
            file("tokenizer.json", 2_480_466, TOKENS_ALL),
            file("model.safetensors", 290_403_936, "07cadb9f25677c8d50df603e66a98fbd842cce45047139baeb16e6219a1e807b"),
        ],
    },
    Model {
        id: "whisper-small.en",
        label: "Small (English)",
        note: "Careful and slower, English only",
        revision: "e8727524f962ee844a7319d92be39ac1bd25655a",
        multilingual: false,
        files: [
            file("config.json", 1943, "3441094addfb2a31ef62ea117e95bd6e437bbad8e93d3697fe420a4a6d3603a5"),
            file("tokenizer.json", 2_405_679, TOKENS_EN),
            file("model.safetensors", 966_992_008, "6014ac49b506df900f66f4aca6b0801eed7245594ace97bcaf73e0ae5b863066"),
        ],
    },
    Model {
        id: "whisper-small",
        label: "Small",
        note: "Careful and slower, many languages",
        revision: "973afd24965f72e36ca33b3055d56a652f456b4d",
        multilingual: true,
        files: [
            file("config.json", 1967, "e6a2b489da1b5aed65a8eb8d1e7466fa867ad5643a8bc138ba708bd56b2875c4"),
            file("tokenizer.json", 2_480_466, TOKENS_ALL),
            file("model.safetensors", 966_995_080, "1d7734884874f1a1513ed9aa760a4f8e97aaa02fd6d93a3a85d27b2ae9ca596b"),
        ],
    },
];

/// The model with this id.
pub fn by_id(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

/// The folder of `model` under `base`.
pub fn dir_of(base: &Path, model: &Model) -> PathBuf {
    base.join(model.id)
}

/// Whether every file of `model` is under `base` and the right size (the contents were checked
/// when the files were downloaded).
pub fn is_installed(base: &Path, model: &Model) -> bool {
    let dir = dir_of(base, model);
    model
        .files
        .iter()
        .all(|f| std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.is_file() && m.len() == f.bytes))
}

/// The SHA-256 of a file, as 64 hexadecimal digits.
pub fn sha256_of(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Downloads what of `model` is not under `base` yet. `progress(done, total)` (bytes of the whole
/// model) gives `false` to stop. A file is kept only when it is the size and has the SHA-256 it
/// should; a stopped or failed download leaves nothing half-written under the file's name.
pub fn download(
    base: &Path,
    model: &Model,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<()> {
    let dir = dir_of(base, model);
    std::fs::create_dir_all(&dir)?;
    let total = model.bytes();
    let mut done = 0u64;
    for f in &model.files {
        let target = dir.join(f.name);
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == f.bytes) {
            done += f.bytes;
            if !progress(done, total) {
                return Err(AsrError::msg("cancelled"));
            }
            continue;
        }
        let url = model.url(f);
        if !url.starts_with(PREFIX) {
            return Err(AsrError::msg("the model is not on the place it is downloaded from"));
        }
        let part = dir.join(format!("{}.part", f.name));
        let _ = std::fs::remove_file(&part);
        let mut child = Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-time", "7200", "-o"])
            .arg(&part)
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| AsrError::msg(format!("curl could not be started: {e}")))?;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(e.into());
                }
            }
            let have = std::fs::metadata(&part).map_or(0, |m| m.len());
            if !progress(done + have.min(f.bytes), total) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&part);
                return Err(AsrError::msg("cancelled"));
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        if !status.success() {
            let _ = std::fs::remove_file(&part);
            return Err(AsrError::msg(format!(
                "downloading {} did not work (is there an internet connection?)",
                f.name
            )));
        }
        let size = std::fs::metadata(&part).map_or(0, |m| m.len());
        if size != f.bytes || sha256_of(&part)? != f.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(AsrError::msg(format!("{} arrived damaged: try again", f.name)));
        }
        std::fs::rename(&part, &target)?;
        done += f.bytes;
        if !progress(done, total) {
            return Err(AsrError::msg("cancelled"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_are_listed_once_and_pinned() {
        let mut ids: Vec<&str> = MODELS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODELS.len());
        for m in &MODELS {
            assert_eq!(m.revision.len(), 40, "{}", m.id);
            assert_eq!(m.multilingual, !m.id.ends_with(".en"), "{}", m.id);
            for f in &m.files {
                assert_eq!(f.sha256.len(), 64, "{} {}", m.id, f.name);
                assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
                let url = m.url(f);
                assert!(url.starts_with("https://huggingface.co/openai/whisper-"), "{url}");
                assert!(url.contains(m.revision) && url.ends_with(f.name), "{url}");
            }
            assert_eq!(m.files[2].name, "model.safetensors");
            assert!(m.bytes() > 100_000_000);
        }
        assert_eq!(by_id("whisper-base").map(|m| m.label), Some("Base"));
        assert!(by_id("whisper-huge").is_none());
        // Smallest first.
        assert!(MODELS.windows(2).all(|w| w[0].bytes() <= w[1].bytes()));
    }

    #[test]
    fn a_model_counts_as_installed_only_with_every_file_the_right_size() {
        let base = std::env::temp_dir().join("picasso_asr_models_test");
        let _ = std::fs::remove_dir_all(&base);
        let m = &MODELS[0];
        assert!(!is_installed(&base, m));
        let dir = dir_of(&base, m);
        std::fs::create_dir_all(&dir).unwrap();
        for f in &m.files {
            // The right names, the wrong sizes.
            std::fs::write(dir.join(f.name), b"x").unwrap();
        }
        assert!(!is_installed(&base, m));
        // The small files at their sizes are not enough without the big one.
        std::fs::write(dir.join("config.json"), vec![0u8; m.files[0].bytes as usize]).unwrap();
        std::fs::write(dir.join("tokenizer.json"), vec![0u8; m.files[1].bytes as usize]).unwrap();
        assert!(!is_installed(&base, m));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_hash_of_a_file_is_the_known_one() {
        let path = std::env::temp_dir().join("picasso_asr_sha_test.txt");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_of(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&path);
        assert!(sha256_of(&path).is_err());
    }
}
