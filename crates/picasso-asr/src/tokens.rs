//! Whisper's words: turning the numbers the model chooses back into text, and finding the special
//! numbers (start, end, language, timestamps) by their names. Read from the model's
//! `tokenizer.json`. Only the way from numbers to text is needed here: the model is never given
//! words, only the special tokens that start it off.

use std::collections::HashMap;

use serde_json::Value;

use crate::{AsrError, Result};

/// The special numbers of a model.
#[derive(Clone, Debug)]
pub struct Tokens {
    /// For each number, the bytes of its piece of text (none for the special ones).
    pieces: Vec<Option<Vec<u8>>>,
    pub sot: u32,
    pub eot: u32,
    pub transcribe: Option<u32>,
    pub no_timestamps: u32,
    /// `<|0.00|>`: the first timestamp; each next one is 20 ms later.
    pub timestamp_begin: u32,
    /// The languages a multilingual model knows: code and token.
    pub languages: Vec<(String, u32)>,
}

/// The letters the byte-level pieces are written with: every byte has one, and the printable
/// ones are themselves.
fn byte_of_char() -> HashMap<char, u8> {
    let mut printable: Vec<u32> = (u32::from(b'!')..=u32::from(b'~')).collect();
    printable.extend(0xA1..=0xAC);
    printable.extend(0xAE..=0xFF);
    let mut map = HashMap::new();
    let mut extra = 0u32;
    for b in 0u32..256 {
        let c = if printable.contains(&b) {
            b
        } else {
            extra += 1;
            255 + extra
        };
        if let (Some(ch), Ok(byte)) = (char::from_u32(c), u8::try_from(b)) {
            map.insert(ch, byte);
        }
    }
    map
}

impl Tokens {
    /// Reads `tokenizer.json`.
    pub fn from_json(text: &str) -> Result<Self> {
        let root: Value = serde_json::from_str(text)?;
        let vocab = root
            .pointer("/model/vocab")
            .and_then(Value::as_object)
            .ok_or_else(|| AsrError::msg("the tokenizer has no vocabulary"))?;
        let added = root
            .get("added_tokens")
            .and_then(Value::as_array)
            .ok_or_else(|| AsrError::msg("the tokenizer has no special tokens"))?;
        let bytes = byte_of_char();
        let size = vocab
            .values()
            .filter_map(Value::as_u64)
            .chain(added.iter().filter_map(|t| t.get("id").and_then(Value::as_u64)))
            .max()
            .map_or(0, |m| m as usize + 1);
        let mut pieces: Vec<Option<Vec<u8>>> = vec![None; size];
        for (piece, id) in vocab {
            let Some(id) = id.as_u64().map(|i| i as usize).filter(|i| *i < size) else {
                continue;
            };
            let decoded: Option<Vec<u8>> = piece.chars().map(|c| bytes.get(&c).copied()).collect();
            pieces[id] = decoded;
        }
        let mut named: HashMap<String, u32> = HashMap::new();
        for t in added {
            let (Some(id), Some(content)) = (
                t.get("id").and_then(Value::as_u64),
                t.get("content").and_then(Value::as_str),
            ) else {
                continue;
            };
            // Special tokens (and the timestamps) are not text.
            if let Some(slot) = pieces.get_mut(id as usize) {
                *slot = None;
            }
            named.insert(content.to_string(), id as u32);
        }
        let need = |name: &str| {
            named
                .get(name)
                .copied()
                .ok_or_else(|| AsrError::msg(format!("the tokenizer has no {name}")))
        };
        let no_timestamps = need("<|notimestamps|>")?;
        // Languages: <|en|>, <|de|>... two or three small letters.
        let mut languages: Vec<(String, u32)> = named
            .iter()
            .filter_map(|(name, id)| {
                let code = name.strip_prefix("<|")?.strip_suffix("|>")?;
                (matches!(code.len(), 2 | 3) && code.chars().all(|c| c.is_ascii_lowercase()))
                    .then(|| (code.to_string(), *id))
            })
            .collect();
        languages.sort_by_key(|(_, id)| *id);
        Ok(Self {
            pieces,
            sot: need("<|startoftranscript|>")?,
            eot: need("<|endoftext|>")?,
            transcribe: named.get("<|transcribe|>").copied(),
            no_timestamps,
            timestamp_begin: no_timestamps + 1,
            languages,
        })
    }

    /// The text of `ids`: the special ones are left out.
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut bytes = Vec::new();
        for id in ids {
            if let Some(Some(piece)) = self.pieces.get(*id as usize) {
                bytes.extend_from_slice(piece);
            }
        }
        String::from_utf8_lossy(&bytes).trim().to_string()
    }

    /// The language token of `code` ("en", "de"...).
    pub fn language(&self, code: &str) -> Option<u32> {
        self.languages.iter().find(|(c, _)| c == code).map(|(_, id)| *id)
    }

    /// Whether the model knows more than one language (it has language tokens to start with).
    pub fn is_multilingual(&self) -> bool {
        !self.languages.is_empty() && self.transcribe.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tokenizer of a few pieces, written as the real file is.
    pub(crate) fn tiny_json() -> String {
        serde_json::json!({
            "added_tokens": [
                {"id": 10, "content": "<|endoftext|>"},
                {"id": 11, "content": "<|startoftranscript|>"},
                {"id": 12, "content": "<|en|>"},
                {"id": 13, "content": "<|de|>"},
                {"id": 14, "content": "<|transcribe|>"},
                {"id": 15, "content": "<|notimestamps|>"},
                {"id": 16, "content": "<|0.00|>"},
                {"id": 17, "content": "<|0.02|>"}
            ],
            "model": {"vocab": {
                "Hello": 0, "Ġworld": 1, "!": 2, "Ġ": 3, "Ã©": 4, "ĠÃ©t": 5, "Ã©t": 6, "Ä": 7
            }}
        })
        .to_string()
    }

    #[test]
    fn the_special_numbers_are_found_by_name() {
        let t = Tokens::from_json(&tiny_json()).unwrap();
        assert_eq!((t.sot, t.eot, t.no_timestamps, t.timestamp_begin), (11, 10, 15, 16));
        assert_eq!(t.transcribe, Some(14));
        assert_eq!(t.language("de"), Some(13));
        assert_eq!(t.language("fr"), None);
        assert!(t.is_multilingual());
        // The special ones are not languages.
        assert_eq!(t.languages.len(), 2);
    }

    #[test]
    fn numbers_become_text_and_the_special_ones_vanish() {
        let t = Tokens::from_json(&tiny_json()).unwrap();
        assert_eq!(t.decode(&[0, 1, 2]), "Hello world!");
        // Special tokens and timestamps add nothing.
        assert_eq!(t.decode(&[11, 12, 14, 16, 0, 17, 10]), "Hello");
        // Letters outside ASCII: "é" is two bytes that arrive as one piece or as two.
        assert_eq!(t.decode(&[4]), "é");
        assert_eq!(t.decode(&[5]), "ét");
        // A number the tokenizer does not have is nothing.
        assert_eq!(t.decode(&[999]), "");
    }

    #[test]
    fn a_tokenizer_that_lacks_what_is_needed_is_refused_in_words() {
        assert!(Tokens::from_json("not json").is_err());
        assert!(Tokens::from_json("{}").is_err());
        let no_sot = r#"{"added_tokens":[{"id":1,"content":"<|endoftext|>"}],"model":{"vocab":{"a":0}}}"#;
        let e = Tokens::from_json(no_sot).unwrap_err().to_string();
        assert!(e.contains("notimestamps") || e.contains("startoftranscript"), "{e}");
    }
}
