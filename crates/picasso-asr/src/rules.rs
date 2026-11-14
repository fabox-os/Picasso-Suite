//! The rules that keep Whisper's timestamps in order while it picks each next token, as in the
//! reference implementation: timestamps come in pairs (before and after a piece of text), never
//! go back in time, the first thing said is a timestamp near the start, and when the timestamps
//! together are likelier than any word, a timestamp is what comes next.

/// What the rules need to know of the model.
#[derive(Clone, Debug)]
pub struct Rules {
    pub eot: u32,
    pub no_timestamps: u32,
    pub timestamp_begin: u32,
    /// Tokens that are never said (the model's own list, and the special ones).
    pub suppress: Vec<u32>,
    /// The first timestamp may be this many steps (of 20 ms) from the window's start, at most.
    pub max_initial_step: u32,
}

impl Rules {
    /// Applies the rules to `logits` (one per token) given `said`, what has been picked in this
    /// window so far.
    pub fn apply(&self, logits: &mut [f32], said: &[u32]) {
        let begin = self.timestamp_begin as usize;
        let eot = self.eot as usize;
        if begin >= logits.len() {
            return;
        }
        let ban = |logits: &mut [f32], range: std::ops::Range<usize>| {
            for l in &mut logits[range.start.min(range.end)..range.end] {
                *l = f32::NEG_INFINITY;
            }
        };
        for id in &self.suppress {
            if let Some(l) = logits.get_mut(*id as usize) {
                *l = f32::NEG_INFINITY;
            }
        }
        if let Some(l) = logits.get_mut(self.no_timestamps as usize) {
            *l = f32::NEG_INFINITY;
        }
        let is_time = |t: u32| t >= self.timestamp_begin;
        let last_was_time = said.last().is_some_and(|t| is_time(*t));
        let before_last_was_time = said.len() < 2 || said.get(said.len() - 2).is_some_and(|t| is_time(*t));
        // Timestamps come in pairs, but for the one before the end.
        if last_was_time {
            if before_last_was_time {
                let len = logits.len();
                ban(logits, begin..len);
            } else {
                ban(logits, 0..eot);
            }
        }
        // They do not go back.
        if let Some(last) = said.iter().rev().find(|t| is_time(**t)) {
            let from = if last_was_time && !before_last_was_time { *last } else { *last + 1 };
            ban(logits, begin..(from as usize).min(logits.len()));
        }
        // The first thing said is a timestamp, not far from the start.
        if said.is_empty() {
            ban(logits, 0..begin);
            let latest = begin + self.max_initial_step as usize;
            if latest + 1 < logits.len() {
                let len = logits.len();
                ban(logits, latest + 1..len);
            }
        }
        // When timestamps are likelier together than any word, say one.
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if max.is_finite() {
            let total = |range: &[f32]| -> f32 {
                let sum: f32 = range.iter().map(|l| (l - max).exp()).sum();
                max + sum.max(f32::MIN_POSITIVE).ln()
            };
            let times = total(&logits[begin..]);
            let words = logits[..begin].iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if times > words {
                ban(logits, 0..begin);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules {
            eot: 5,
            no_timestamps: 6,
            timestamp_begin: 7,
            suppress: vec![3],
            max_initial_step: 2,
        }
    }

    /// Logits of 12 tokens (0..=4 words, 5 end, 6 no-timestamps, 7..=11 timestamps), all equal.
    fn flat() -> Vec<f32> {
        vec![0.0; 12]
    }

    fn allowed(l: &[f32]) -> Vec<usize> {
        (0..l.len()).filter(|i| l[*i].is_finite()).collect()
    }

    #[test]
    fn the_first_thing_said_is_a_timestamp_near_the_start() {
        let mut l = flat();
        rules().apply(&mut l, &[]);
        // Timestamps 0, 1 and 2 (steps from the start): 7, 8, 9.
        assert_eq!(allowed(&l), vec![7, 8, 9]);
    }

    #[test]
    fn a_timestamp_after_text_comes_in_a_pair_and_never_goes_back() {
        // A timestamp (8), some text (1): next may be a word, the end or a timestamp not before 8.
        let mut l = flat();
        l[8] = 1.0;
        rules().apply(&mut l, &[8, 1]);
        assert!(allowed(&l).iter().all(|i| *i == 5 || *i < 3 || *i == 4 || *i >= 9), "{:?}", allowed(&l));
        assert!(!allowed(&l).contains(&7) && !allowed(&l).contains(&8));
        // Just after a lone timestamp it must be a timestamp or the end: no words.
        let mut l = flat();
        rules().apply(&mut l, &[8, 1, 9]);
        assert!(allowed(&l).iter().all(|i| *i == 5 || *i >= 9), "{:?}", allowed(&l));
        // After a pair of timestamps it is words again (or the next window's timestamps are banned).
        let mut l = flat();
        rules().apply(&mut l, &[8, 1, 9, 9]);
        assert!(allowed(&l).iter().all(|i| *i < 6 && *i != 3), "{:?}", allowed(&l));
    }

    #[test]
    fn timestamps_that_add_up_to_more_than_any_word_win() {
        let mut l = flat();
        // Five timestamps at the same level against one word at that level: together likelier.
        l[1] = 0.0;
        l[7..12].fill(0.0);
        rules().apply(&mut l, &[8, 1]);
        assert!(allowed(&l).iter().all(|i| *i >= 9), "{:?}", allowed(&l));
        // A word that is far likelier stays.
        let mut l = flat();
        l[0] = 20.0;
        rules().apply(&mut l, &[8, 1]);
        assert!(allowed(&l).contains(&0));
    }

    #[test]
    fn the_suppressed_and_the_no_timestamp_token_are_never_picked() {
        let mut l = flat();
        l[3] = 100.0;
        l[6] = 100.0;
        rules().apply(&mut l, &[8, 1]);
        assert!(!allowed(&l).contains(&3) && !allowed(&l).contains(&6));
        // Short logits do not panic.
        rules().apply(&mut [0.0; 3], &[]);
    }
}
