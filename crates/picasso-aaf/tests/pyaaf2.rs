//! The AAF reader on a file it did not write: `data/pyaaf2-edit.aaf`, written by pyaaf2 (see
//! `data/make_pyaaf2_sample.py` for what is in it), laid out as Avid Media Composer lays out an
//! edit (master mobs, file source mobs with locators, tape mobs at 10:00:00:00).

use picasso_core::otio::{OtioMedia, OtioTrack};
use picasso_core::{Fps, TrackKind, Transition, TransitionKind};

const SAMPLE: &[u8] = include_bytes!("data/pyaaf2-edit.aaf");

fn places(track: &OtioTrack) -> Vec<(u64, u64, u64)> {
    track
        .clips
        .iter()
        .map(|c| (c.record, c.duration, c.source_in))
        .collect()
}

fn file(path: &str) -> OtioMedia {
    OtioMedia::File { path: path.into() }
}

#[test]
fn an_edit_written_by_pyaaf2_is_read() {
    let t = picasso_aaf::read_aaf(SAMPLE, Fps { num: 25, den: 1 }).unwrap();
    assert_eq!(t.name, "Edit from pyaaf2");
    let kinds: Vec<TrackKind> = t.tracks.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, [TrackKind::Video, TrackKind::Audio]);
    let (v1, a1) = (&t.tracks[0], &t.tracks[1]);

    // Shot A ends at the dissolve's cut (5 frames into it) and shot B starts there, with the
    // dissolve's frames before the cut as its handle; after a 25-frame filler, shot A again from
    // inside its effect, then the nested composition as a compound clip.
    assert_eq!(
        places(v1),
        [(0, 45, 10), (45, 35, 5), (105, 30, 100), (135, 20, 0)]
    );
    assert_eq!(t.skipped, 0, "the nested composition is read");
    assert_eq!(v1.clips[0].transition, None);
    assert_eq!(
        v1.clips[1].transition,
        Some(Transition {
            kind: TransitionKind::CrossDissolve,
            frames: 10
        })
    );
    // Each clip is found through its master and file source mob, to the file's address; the
    // name is the master clip's, as the editor shows it.
    assert_eq!(v1.clips[0].media, file("C:/Media/shot A.mov"));
    assert_eq!(v1.clips[0].media_name, "shot A.mov");
    assert_eq!(v1.clips[0].name, "Shot A");
    assert_eq!(v1.clips[0].available, Some(600));
    assert_eq!(v1.clips[1].media, file("/Volumes/Media/B.mxf"));
    assert_eq!(v1.clips[2].media, file("C:/Media/shot A.mov"));
    // The nested composition is a compound clip: its own track, with shot B in it.
    let nested = &v1.clips[3].nested;
    assert_eq!(nested.len(), 1, "the nested composition's one track");
    assert_eq!(nested[0].kind, TrackKind::Video);
    assert_eq!(places(&nested[0]), [(0, 20, 0)]);
    assert_eq!(nested[0].clips[0].media, file("/Volumes/Media/B.mxf"));
    assert_eq!(nested[0].clips[0].name, "Shot B");

    // The sound's master counts 48 kHz samples: 19200 of them is frame 10 at 25 fps, and the
    // file's 60 seconds are 1500 frames.
    assert_eq!(places(a1), [(0, 50, 10)]);
    assert_eq!(a1.clips[0].media, file("C:/Media/sound.wav"));
    assert_eq!(a1.clips[0].available, Some(1500));
}

#[test]
fn the_same_edit_is_read_at_another_frame_rate() {
    let t = picasso_aaf::read_aaf(SAMPLE, Fps { num: 50, den: 1 }).unwrap();
    assert_eq!(places(&t.tracks[0])[0], (0, 90, 20));
    assert_eq!(places(&t.tracks[1]), [(0, 100, 20)]);
}
