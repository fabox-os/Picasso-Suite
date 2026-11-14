//! From what the model says to subtitles: a window of tokens is cut into segments (a piece of
//! text between two timestamps), and the segments of a whole sound are cut into cues of a few
//! words each, with times, that do not overlap.

/// A piece of text between two times, in seconds from the start of the sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// One subtitle: when it shows and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// What one window of 30 seconds said.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// Seconds from the window's start.
    pub segments: Vec<Segment>,
    /// How far into the window the next one should start, in seconds.
    pub advance: f64,
}

/// The longest a window is, in seconds.
pub const WINDOW_SECONDS: f64 = 30.0;
/// A step of the timestamps, in seconds.
pub const STEP: f64 = 0.02;

/// Reads the `said` tokens of one window: `timestamp_begin` and `eot` are the model's numbers,
/// `text` turns text tokens into words.
pub fn parse_window(
    said: &[u32],
    timestamp_begin: u32,
    eot: u32,
    text: &dyn Fn(&[u32]) -> String,
) -> Window {
    let mut segments = Vec::new();
    let mut start: Option<f64> = None;
    let mut words: Vec<u32> = Vec::new();
    let mut cut_short = false;
    for &t in said {
        if t == eot {
            break;
        }
        if t >= timestamp_begin {
            let at = f64::from(t - timestamp_begin) * STEP;
            match start {
                None => start = Some(at),
                Some(s) if !words.is_empty() => {
                    let body = text(&words);
                    if !body.is_empty() {
                        segments.push(Segment {
                            start: s,
                            end: at.max(s),
                            text: body,
                        });
                    }
                    words.clear();
                    start = None;
                }
                // Two timestamps in a row with nothing between: the later one is the start.
                Some(_) => start = Some(at),
            }
        } else {
            words.push(t);
        }
    }
    if !words.is_empty() {
        // Text with no timestamp after it: it runs to the end of the window.
        let body = text(&words);
        if !body.is_empty() {
            let s = start.unwrap_or(0.0);
            segments.push(Segment {
                start: s,
                end: WINDOW_SECONDS.max(s),
                text: body,
            });
        }
        cut_short = true;
    }
    let advance = match segments.last() {
        Some(last) if !cut_short && last.end >= 1.0 => last.end.min(WINDOW_SECONDS),
        _ => WINDOW_SECONDS,
    };
    Window { segments, advance }
}

/// The words a model says over and over when it has lost its way: the same text three times in
/// a row is cut to one.
pub fn drop_repeats(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    let mut run = 0;
    for s in segments {
        if out.last().is_some_and(|l| l.text.eq_ignore_ascii_case(&s.text)) {
            run += 1;
            if run >= 2 {
                continue;
            }
        } else {
            run = 0;
        }
        out.push(s);
    }
    out
}

/// The cues for `segments` (in order, seconds): each at most `max_words` words and
/// `max_seconds` long, the times of the words of a segment shared out by their length.
pub fn pack(segments: &[Segment], max_words: usize, max_seconds: f64) -> Vec<Cue> {
    let max_words = max_words.max(1);
    let max_seconds = max_seconds.max(1.0);
    let mut cues: Vec<Cue> = Vec::new();
    for s in segments {
        let words: Vec<&str> = s.text.split_whitespace().collect();
        if words.is_empty() || !s.start.is_finite() || !s.end.is_finite() {
            continue;
        }
        let length = (s.end - s.start).max(0.0);
        // As few cues as the limits allow, with about as many words in each.
        let by_words = words.len().div_ceil(max_words);
        let by_time = (length / max_seconds).ceil() as usize;
        let count = by_words.max(by_time).clamp(1, words.len());
        let weight = |w: &str| w.chars().count() as f64 + 1.0;
        let total: f64 = words.iter().map(|w| weight(w)).sum();
        let mut at = s.start;
        let mut taken = 0.0;
        // `count` chunks of nearly the same number of words.
        let (base, extra) = (words.len() / count, words.len() % count);
        let mut first = 0;
        for i in 0..count {
            let size = base + usize::from(i < extra);
            let chunk = &words[first..first + size];
            first += size;
            let share: f64 = chunk.iter().map(|w| weight(w)).sum();
            taken += share;
            let end = s.start + length * (taken / total);
            cues.push(Cue {
                start_ms: (at * 1000.0).round() as u64,
                end_ms: (end * 1000.0).round().max((at * 1000.0).round()) as u64,
                text: chunk.join(" "),
            });
            at = end;
        }
    }
    // In order, not overlapping, and none too short to read where there is room.
    cues.sort_by_key(|c| c.start_ms);
    for i in 0..cues.len() {
        if let Some(next) = cues.get(i + 1).map(|c| c.start_ms) {
            if cues[i].end_ms > next {
                cues[i].end_ms = next;
            }
        }
        let room = cues.get(i + 1).map_or(u64::MAX, |c| c.start_ms);
        if cues[i].end_ms < cues[i].start_ms + 400 {
            cues[i].end_ms = (cues[i].start_ms + 400).min(room);
        }
    }
    cues.retain(|c| c.end_ms > c.start_ms);
    cues
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: u32 = 100;
    const EOT: u32 = 50;

    /// Tokens 1..=9 are words "a".."i".
    fn text(ids: &[u32]) -> String {
        ids.iter()
            .map(|i| char::from(b'a' + (*i as u8 - 1)).to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_window_is_cut_into_segments_between_timestamps() {
        // <0.00> a b <2.00> <2.00> c <4.00> eot
        let said = [TS, 1, 2, TS + 100, TS + 100, 3, TS + 200, EOT];
        let w = parse_window(&said, TS, EOT, &text);
        assert_eq!(
            w.segments,
            vec![
                Segment { start: 0.0, end: 2.0, text: "a b".into() },
                Segment { start: 2.0, end: 4.0, text: "c".into() },
            ]
        );
        // Ends cleanly at 4 seconds: the next window starts there.
        assert_eq!(w.advance, 4.0);
    }

    #[test]
    fn text_that_is_cut_short_runs_to_the_end_and_the_whole_window_is_used() {
        let w = parse_window(&[TS + 50, 4, 5], TS, EOT, &text);
        assert_eq!(w.segments, vec![Segment { start: 1.0, end: 30.0, text: "d e".into() }]);
        assert_eq!(w.advance, WINDOW_SECONDS);
        // Nothing said: nothing, and on to the next window.
        let none = parse_window(&[EOT], TS, EOT, &text);
        assert!(none.segments.is_empty());
        assert_eq!(none.advance, WINDOW_SECONDS);
        // A segment that ends before a second does not hold the reading back to a standstill.
        let early = parse_window(&[TS, 1, TS + 10, EOT], TS, EOT, &text);
        assert_eq!(early.advance, WINDOW_SECONDS);
    }

    #[test]
    fn the_same_words_over_and_over_are_cut_down() {
        let seg = |t: &str, s: f64| Segment { start: s, end: s + 1.0, text: t.into() };
        let said = vec![seg("Thank you.", 0.0), seg("Thank you.", 1.0), seg("Thank you.", 2.0), seg("Thank you.", 3.0), seg("Goodbye", 4.0), seg("Goodbye", 5.0)];
        let cut = drop_repeats(said);
        let texts: Vec<&str> = cut.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Thank you.", "Thank you.", "Goodbye", "Goodbye"]);
    }

    #[test]
    fn long_segments_are_cut_into_cues_that_share_the_time_by_length() {
        let seg = Segment { start: 10.0, end: 16.0, text: "one two three four five six seven eight".into() };
        let cues = pack(&[seg], 4, 6.0);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "one two three four");
        assert_eq!(cues[1].text, "five six seven eight");
        assert_eq!(cues[0].start_ms, 10_000);
        assert_eq!(cues[1].end_ms, 16_000);
        // They meet where the first ends, and the first took its share of the letters.
        assert_eq!(cues[0].end_ms, cues[1].start_ms);
        assert!((cues[0].end_ms as i64 - 13_000).abs() < 800, "{}", cues[0].end_ms);
        // A short segment stays one cue.
        let one = pack(&[Segment { start: 0.0, end: 2.0, text: "Hello there".into() }], 7, 6.0);
        assert_eq!(one, vec![Cue { start_ms: 0, end_ms: 2000, text: "Hello there".into() }]);
        // A long time cuts a segment that has few words.
        let slow = pack(&[Segment { start: 0.0, end: 12.0, text: "a b c d".into() }], 7, 5.0);
        assert!(slow.len() >= 3, "{slow:?}");
    }

    #[test]
    fn cues_do_not_overlap_and_are_not_too_short_where_there_is_room() {
        let a = Segment { start: 0.0, end: 0.1, text: "Hi".into() };
        let b = Segment { start: 2.0, end: 3.0, text: "there".into() };
        let c = Segment { start: 3.0, end: 3.05, text: "now".into() };
        let d = Segment { start: 3.1, end: 4.0, text: "ok".into() };
        let cues = pack(&[a, b, c, d], 7, 6.0);
        assert_eq!(cues.len(), 4);
        assert!(cues[0].end_ms >= 400, "a short one is made readable");
        for w in cues.windows(2) {
            assert!(w[0].end_ms <= w[1].start_ms, "{w:?}");
        }
        // Nothing, or broken times, make nothing.
        assert!(pack(&[], 7, 6.0).is_empty());
        assert!(pack(&[Segment { start: f64::NAN, end: 1.0, text: "x".into() }], 7, 6.0).is_empty());
    }
}
