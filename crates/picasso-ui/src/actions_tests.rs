//! Tests of the menu acts, run against a real `StudioApp` (no frames needed).

use super::*;

/// An app showing the demo project: three 96-frame clips on V1 at 0, 96 and 192.
fn app() -> StudioApp {
    let ctx = Context::default();
    let mut app = StudioApp::new(&ctx);
    app.load_demo();
    app
}

fn starts(app: &StudioApp) -> Vec<(u64, u64)> {
    app.project().tracks[0]
        .clips
        .iter()
        .map(|c| (c.start, c.duration))
        .collect()
}

fn act(app: &mut StudioApp, act: Act) {
    app.do_act(&Context::default(), act);
}

#[test]
fn nothing_is_enabled_that_has_nothing_to_act_on() {
    let app = app();
    for a in [
        Act::Cut,
        Act::Copy,
        Act::Paste,
        Act::PasteMix,
        Act::Duplicate,
        Act::DeleteSelected,
        Act::DeselectAll,
        Act::ClearIn,
        Act::ModifyMarker,
        Act::PlayInToOut,
        Act::Undo,
    ] {
        assert!(!app.act_enabled(a), "{a:?}");
    }
    for a in [
        Act::SelectAll,
        Act::SaveProject,
        Act::PlayForward,
        Act::AddMarker,
        Act::SplitClip,
    ] {
        assert!(app.act_enabled(a), "{a:?}");
    }
}

#[test]
fn select_all_copy_paste_is_one_step_and_lands_after_the_playhead() {
    let mut app = app();
    act(&mut app, Act::SelectAll);
    assert_eq!(app.selected_ids().len(), 3);
    act(&mut app, Act::Copy);
    app.go_to(u64::MAX);
    act(&mut app, Act::Paste);
    assert_eq!(
        starts(&app),
        vec![
            (0, 96),
            (96, 96),
            (192, 96),
            (288, 96),
            (384, 96),
            (480, 96)
        ]
    );
    assert_eq!(app.selected_ids().len(), 3, "the pasted clips are selected");
    assert!(app.session.undo());
    assert_eq!(starts(&app).len(), 3, "one undo takes back all three");
}

#[test]
fn a_paste_that_does_not_fit_changes_nothing() {
    let mut app = app();
    act(&mut app, Act::SelectAll);
    act(&mut app, Act::Copy);
    app.go_to(10);
    let before = app.project().clone();
    act(&mut app, Act::Paste);
    assert_eq!(*app.project(), before);
    assert!(app.status.error);
}

#[test]
fn paste_keeps_what_the_clip_looked_like() {
    let mut app = app();
    let first = app.project().tracks[0].clips[0].id;
    app.run(Command::SetOpacity {
        clip: first,
        opacity: 0.4,
    });
    app.run(Command::SetGrade {
        clip: first,
        grade: ColorGrade {
            saturation: 0.3,
            ..Default::default()
        },
        node: 0,
    });
    app.run(Command::SetClipEnabled {
        clip: first,
        enabled: false,
    });
    app.select_only(first);
    act(&mut app, Act::Copy);
    app.go_to(u64::MAX);
    act(&mut app, Act::Paste);
    let pasted = app.project().tracks[0].clips.last().unwrap().clone();
    assert_eq!(
        (pasted.opacity, pasted.enabled, pasted.grade.saturation),
        (0.4, false, 0.3)
    );
    assert_eq!(pasted.start, 288);
}

#[test]
fn paste_mix_leaves_the_picture_alone_and_puts_the_sound_on_a1() {
    let (mut app, picture, _sound) = app_with_linked_video();
    let (v2, media) = {
        let (track, clip) = app.project().clip(picture).unwrap();
        (track.id, clip.media)
    };
    app.select_only(picture);
    act(&mut app, Act::Copy);
    app.go_to(200);
    act(&mut app, Act::PasteMix);
    assert!(!app.status.error, "{}", app.status.text);
    // The picture is not pasted a second time, and its sound is where the playhead was.
    assert_eq!(app.project().track(v2).unwrap().clips.len(), 1);
    let (id, start, length) = {
        let a1 = app
            .project()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        let pasted = a1
            .clips
            .iter()
            .find(|c| c.start == 200)
            .expect("the sound at the playhead");
        assert_eq!((pasted.media, pasted.duration, pasted.sound), (media, 96, true));
        (pasted.id, pasted.start, pasted.duration)
    };
    assert_eq!((start, length), (200, 96));
    assert_eq!(app.selected_ids(), vec![id], "the pasted sound is selected");
    // And the mixer hears it there: the picture is silent, the two sound clips play.
    let sounds = picasso_audio::plan(app.project()).clips;
    assert_eq!(sounds.len(), 2);
    assert!(sounds
        .iter()
        .any(|s| s.start == picasso_audio::frames_to_samples(app.project(), 200)));
    assert!(app.session.undo());
    assert!(
        app.project()
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .all(|t| t.clips.iter().all(|c| c.id != id)),
        "one undo takes the pasted sound back"
    );
}

#[test]
fn paste_mix_goes_on_the_sound_track_of_the_selected_clip() {
    let (mut app, picture, _sound) = app_with_linked_video();
    let Some(Outcome::Track(a2)) = app.run(Command::AddTrack {
        kind: TrackKind::Audio,
    }) else {
        panic!("a second sound track")
    };
    let media = app.project().clip(picture).unwrap().1.media;
    let Some(Outcome::Clip(on_a2)) = app.run(Command::AddClip {
        track: a2,
        media,
        start: 400,
        duration: None,
        with_sound: false,
    }) else {
        panic!("a clip on A2")
    };
    app.select_only(picture);
    act(&mut app, Act::Copy);
    // The selected clip is on A2, so the sound goes there, not to the first sound track.
    app.select_only(on_a2);
    app.go_to(500);
    let at = app.playhead;
    act(&mut app, Act::PasteMix);
    assert!(!app.status.error, "{}", app.status.text);
    let on = |app: &StudioApp, track: TrackId| {
        app.project()
            .track(track)
            .is_some_and(|t| t.clips.iter().any(|c| c.start == at))
    };
    assert!(on(&app, a2));
    assert!(app
        .project()
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && t.id != a2)
        .all(|t| !on(&app, t.id)));
}

#[test]
fn paste_mix_says_when_there_is_no_sound_track() {
    let mut app = app();
    let clip = app.project().tracks[0].clips[0].id;
    app.select_only(clip);
    act(&mut app, Act::Copy);
    act(&mut app, Act::PasteMix);
    assert!(app.status.error);
    assert!(
        app.status.text.contains("no sound track"),
        "{}",
        app.status.text
    );
    assert_eq!(app.project().duration(), 288, "nothing was changed");
}

#[test]
fn paste_mix_says_so_when_every_sound_track_is_locked() {
    let (mut app, picture, _sound) = app_with_linked_video();
    let a1 = app
        .project()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio)
        .unwrap()
        .id;
    app.run(Command::SetTrackLocked {
        track: a1,
        locked: true,
    });
    app.select_only(picture);
    act(&mut app, Act::Copy);
    let before = app.project().clone();
    act(&mut app, Act::PasteMix);
    assert!(app.status.error);
    assert!(
        app.status.text.contains("locked"),
        "{}",
        app.status.text
    );
    assert_eq!(*app.project(), before, "nothing was changed");
}

#[test]
fn duplicate_goes_right_after_the_selection() {
    let mut app = app();
    let first = app.project().tracks[0].clips[0].id;
    app.select_only(first);
    // The next clip is in the way.
    let before = app.project().clone();
    act(&mut app, Act::Duplicate);
    assert_eq!(*app.project(), before);
    app.select_only(app.project().tracks[0].clips[2].id);
    act(&mut app, Act::Duplicate);
    assert_eq!(starts(&app).last(), Some(&(288, 96)));
}

#[test]
fn delete_removes_every_selected_clip_and_ripple_closes_the_gaps() {
    let mut app = app();
    act(&mut app, Act::SelectAll);
    act(&mut app, Act::DeleteSelected);
    assert!(starts(&app).is_empty());
    assert!(app.selection.is_none());
    app.session.undo();
    let ids: Vec<ClipId> = app.project().tracks[0].clips.iter().map(|c| c.id).collect();
    app.select_many(vec![ids[0], ids[1]]);
    act(&mut app, Act::RippleDeleteSelected);
    assert_eq!(
        starts(&app),
        vec![(0, 96)],
        "the third clip moved to the start"
    );
    assert_eq!(app.playhead, 0, "the playhead follows the cut");
}

#[test]
fn cut_copies_then_deletes() {
    let mut app = app();
    app.select_only(app.project().tracks[0].clips[1].id);
    act(&mut app, Act::Cut);
    assert_eq!(starts(&app), vec![(0, 96), (192, 96)]);
    app.go_to(96);
    act(&mut app, Act::Paste);
    assert_eq!(starts(&app), vec![(0, 96), (96, 96), (192, 96)]);
}

#[test]
fn ripple_start_and_end_trim_to_the_playhead_and_close_the_gap() {
    let mut app = app();
    let first = app.project().tracks[0].clips[0].id;
    app.select_only(first);
    app.go_to(30);
    act(&mut app, Act::RippleStart);
    assert_eq!(starts(&app), vec![(0, 66), (66, 96), (162, 96)]);
    assert_eq!(app.project().clip(first).unwrap().1.source_in, 30);
    app.session.undo();
    assert_eq!(
        starts(&app),
        vec![(0, 96), (96, 96), (192, 96)],
        "one undo takes the whole ripple back"
    );
    act(&mut app, Act::RippleEnd);
    assert_eq!(starts(&app), vec![(0, 30), (30, 96), (126, 96)]);
}

#[test]
fn ripple_trim_needs_the_playhead_inside_the_clip() {
    let mut app = app();
    app.select_only(app.project().tracks[0].clips[0].id);
    app.go_to(96);
    let before = app.project().clone();
    act(&mut app, Act::RippleEnd);
    assert_eq!(*app.project(), before);
    assert!(app.status.error);
}

#[test]
fn trim_to_the_playhead_and_extend_edit() {
    let mut app = app();
    let second = app.project().tracks[0].clips[1].id;
    app.select_only(second);
    app.go_to(150);
    act(&mut app, Act::TrimEndToPlayhead);
    assert_eq!(app.project().clip(second).unwrap().1.end(), 150);
    app.go_to(180);
    act(&mut app, Act::ExtendEdit);
    assert_eq!(app.project().clip(second).unwrap().1.end(), 180);
    app.go_to(120);
    act(&mut app, Act::TrimStartToPlayhead);
    assert_eq!(app.project().clip(second).unwrap().1.start, 120);
    app.go_to(130);
    let before = app.project().clone();
    act(&mut app, Act::ExtendEdit);
    assert_eq!(
        *app.project(),
        before,
        "inside the clip there is nothing to extend"
    );
}

#[test]
fn fit_to_fill_needs_marks_and_place_on_top_stacks() {
    let mut app = app();
    let media = app.project().media[0].id;
    app.selected_media = Some(media);
    let before = app.project().clone();
    act(&mut app, Act::FitToFill);
    assert_eq!(*app.project(), before, "without In and Out nothing happens");
    app.run(Command::SetMarks {
        mark_in: Some(10),
        mark_out: Some(30),
    });
    act(&mut app, Act::FitToFill);
    let id = app.selection.unwrap();
    let (_, c) = app.project().clip(id).unwrap();
    assert_eq!((c.start, c.duration), (10, 20));
    app.go_to(15);
    let tracks = app.project().tracks.len();
    act(&mut app, Act::PlaceOnTop);
    let top = app.selection.unwrap();
    assert_ne!(top, id);
    assert!(app.project().tracks.len() >= tracks);
    assert_eq!(app.project().clip(top).unwrap().1.start, 15);
}

#[test]
fn freeze_and_reverse_from_the_clip_menu() {
    let mut app = app();
    let second = app.project().tracks[0].clips[1].id;
    app.select_only(second);
    act(&mut app, Act::ReverseClip);
    assert!(app.project().clip(second).unwrap().1.reverse);
    act(&mut app, Act::FreezeFrame);
    assert!(app.project().clip(second).unwrap().1.freeze);
    // Copied and pasted, the clip keeps both.
    act(&mut app, Act::Copy);
    app.go_to(5000);
    act(&mut app, Act::Paste);
    let pasted = app.project().tracks[0].clips.last().unwrap();
    assert!(pasted.reverse && pasted.freeze && pasted.id != second);
    act(&mut app, Act::FreezeFrame);
    act(&mut app, Act::ReverseClip);
    let pasted = app.project().tracks[0].clips.last().unwrap();
    assert!(!pasted.reverse && !pasted.freeze);
}

#[test]
fn paste_insert_pushes_the_rest_later() {
    let mut app = app();
    let first = app.project().tracks[0].clips[0].clone();
    let end = app.project().tracks[0].clips.last().unwrap().end();
    app.select_only(first.id);
    act(&mut app, Act::Copy);
    app.go_to(first.end());
    act(&mut app, Act::PasteInsert);
    let clips = &app.project().tracks[0].clips;
    assert_eq!(clips.last().unwrap().end(), end + first.duration);
    assert_eq!(clips[1].start, first.end());
    assert_eq!(clips[1].media, first.media);
    assert!(clips.windows(2).all(|w| w[0].end() <= w[1].start));
}

#[test]
fn grab_a_still_and_apply_its_grade() {
    let mut app = app();
    let [a, b] = [0, 1].map(|i| app.project().tracks[0].clips[i].clone());
    app.go_to(a.start + 2);
    let warm = picasso_color::ColorGrade {
        saturation: 1.5,
        ..Default::default()
    };
    app.run(Command::SetGrade {
        clip: a.id,
        grade: warm,
        node: 0,
    });
    act(&mut app, Act::GrabStill);
    assert_eq!(app.project().stills.len(), 1);
    let png = picasso_core::base64::decode(&app.project().stills[0].png).unwrap();
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    app.go_to(b.start + 2);
    app.select_only(b.id);
    let still = app.project().stills[0].id;
    app.run(Command::ApplyStill { still, clip: b.id });
    assert_eq!(app.project().clip(b.id).unwrap().1.grade.saturation, 1.5);
    act(&mut app, Act::PlayStill);
    assert!(app.color.playing_still);
}

#[test]
fn a_title_from_the_effects_menu() {
    let mut app = app();
    app.go_to(10);
    act(&mut app, Act::AddTitle);
    let id = app.selection.unwrap();
    let (_, clip) = app.project().clip(id).unwrap();
    assert_eq!(clip.start, 10);
    let media = app.project().media(clip.media).unwrap();
    assert!(
        matches!(&media.source, picasso_core::MediaSource::Title(t) if t.text == "Basic Title")
    );
    // One undo removes both the clip and its media.
    let items = app.project().media.len();
    app.undo();
    assert_eq!(app.project().media.len(), items - 1);
}

#[test]
fn transitions_from_the_menus() {
    let mut app = app();
    let second = app.project().tracks[0].clips[1].clone();
    app.go_to(second.start + 3);
    assert!(app.act_enabled(Act::AddCrossDissolve));
    assert!(!app.act_enabled(Act::RemoveTransition));
    act(&mut app, Act::AddCrossDissolve);
    let t = app.project().clip(second.id).unwrap().1.transition.unwrap();
    assert_eq!(t.kind, picasso_core::TransitionKind::CrossDissolve);
    act(&mut app, Act::RemoveTransition);
    assert!(app
        .project()
        .clip(second.id)
        .unwrap()
        .1
        .transition
        .is_none());
    act(&mut app, Act::AddTransitionToAll);
    let with: usize = app.project().tracks[0]
        .clips
        .iter()
        .filter(|c| c.transition.is_some())
        .count();
    assert_eq!(with, 2, "both cuts of the demo timeline");
}

#[test]
fn compound_clips_from_the_menu() {
    let mut app = app();
    let clips: Vec<_> = app.project().tracks[0].clips.iter().map(|c| c.id).collect();
    assert!(!app.act_enabled(Act::DecomposeCompound));
    app.select_only(clips[1]);
    app.selected_more.insert(clips[2]);
    act(&mut app, Act::CreateCompound);
    let compound = app.selection.unwrap();
    assert_eq!(app.project().tracks[0].clips.len(), 2);
    assert!(app
        .project()
        .media
        .iter()
        .any(|m| m.name == "Compound Clip 1"));
    assert!(app.act_enabled(Act::DecomposeCompound));
    act(&mut app, Act::DecomposeCompound);
    assert_eq!(app.project().tracks[0].clips.len(), 3);
    assert!(app.project().clip(compound).is_none());
}

#[test]
fn new_switch_and_delete_timelines() {
    let mut app = app();
    let clips = app.project().tracks[0].clips.len();
    assert!(!app.act_enabled(Act::SwitchTimeline));
    act(&mut app, Act::NewTimeline);
    assert_eq!(app.project().timeline_name, "Timeline 2");
    assert!(app.project().tracks.iter().all(|t| t.clips.is_empty()));
    act(&mut app, Act::SwitchTimeline);
    assert_eq!(app.project().timeline_name, "Timeline 1");
    assert_eq!(app.project().tracks[0].clips.len(), clips);
    act(&mut app, Act::DeleteTimeline);
    assert_eq!(app.project().timeline_name, "Timeline 2");
    assert!(
        !app.act_enabled(Act::DeleteTimeline),
        "the last timeline stays"
    );
    app.undo();
    assert_eq!(app.project().timeline_list().len(), 2);
}

#[test]
fn slip_and_slide_by_one_frame() {
    let mut app = app();
    let second = app.project().tracks[0].clips[1].id;
    app.select_only(second);
    let before = app.project().clip(second).unwrap().1.clone();
    act(&mut app, Act::SlideRight);
    let after = app.project().clip(second).unwrap().1.clone();
    assert_eq!(after.start, before.start + 1);
    assert_eq!(
        after.source_in, before.source_in,
        "a slide keeps what the clip shows"
    );
    act(&mut app, Act::SlipRight);
    let slipped = app.project().clip(second).unwrap().1.clone();
    assert_eq!(slipped.start, after.start, "a slip keeps the clip in place");
    assert_eq!(slipped.source_in, after.source_in + 1);
    act(&mut app, Act::SlipLeft);
    act(&mut app, Act::SlideLeft);
    assert_eq!(app.project().clip(second).unwrap().1, &before);
}

#[test]
fn in_and_out_marks_keep_in_before_out() {
    let mut app = app();
    app.go_to(10);
    act(&mut app, Act::MarkIn);
    app.go_to(20);
    act(&mut app, Act::MarkOut);
    assert_eq!(
        (app.project().mark_in, app.project().mark_out),
        (Some(10), Some(21))
    );
    app.go_to(30);
    act(&mut app, Act::MarkIn);
    assert_eq!(
        (app.project().mark_in, app.project().mark_out),
        (Some(30), None),
        "an In after the Out clears the Out"
    );
    app.go_to(5);
    act(&mut app, Act::MarkOut);
    assert_eq!(
        (app.project().mark_in, app.project().mark_out),
        (None, Some(6)),
        "an Out before the In clears the In"
    );
    act(&mut app, Act::ClearInOut);
    assert_eq!(
        (app.project().mark_in, app.project().mark_out),
        (None, None)
    );
}

#[test]
fn mark_clip_and_go_to_in_out() {
    let mut app = app();
    app.select_only(app.project().tracks[0].clips[1].id);
    act(&mut app, Act::MarkClip);
    assert_eq!(
        (app.project().mark_in, app.project().mark_out),
        (Some(96), Some(192))
    );
    act(&mut app, Act::GoOut);
    assert_eq!(app.playhead, 191);
    act(&mut app, Act::GoIn);
    assert_eq!(app.playhead, 96);
    act(&mut app, Act::ClearOut);
    assert_eq!(app.project().mark_out, None);
}

#[test]
fn markers_can_be_added_found_and_removed() {
    let mut app = app();
    for at in [20, 100] {
        app.go_to(at);
        act(&mut app, Act::AddMarker);
    }
    app.go_to(100);
    let before = app.project().clone();
    act(&mut app, Act::AddMarker);
    assert_eq!(
        *app.project(),
        before,
        "a second marker on the same frame is refused"
    );
    assert!(app.status.error);
    act(&mut app, Act::PreviousMarker);
    assert_eq!(app.playhead, 20);
    act(&mut app, Act::NextMarker);
    assert_eq!(app.playhead, 100);
    act(&mut app, Act::DeleteMarkerHere);
    assert_eq!(app.project().markers.len(), 1);
    act(&mut app, Act::DeleteAllMarkers);
    assert!(app.project().markers.is_empty());
    assert!(!app.act_enabled(Act::NextMarker));
}

#[test]
fn the_marker_dialog_opens_on_the_marker_under_the_playhead() {
    let mut app = app();
    app.go_to(40);
    act(&mut app, Act::ModifyMarker);
    assert!(
        app.marker_dialog.is_none(),
        "there is no marker to modify yet"
    );
    act(&mut app, Act::AddMarkerDialog);
    assert!(matches!(&app.marker_dialog, Some(d) if d.frame == 40 && !d.exists));
    app.marker_dialog = None;
    app.run(Command::SetMarker {
        frame: 40,
        name: "cue".into(),
        color: 3,
    });
    act(&mut app, Act::ModifyMarker);
    assert!(matches!(&app.marker_dialog, Some(d) if d.exists && d.name == "cue" && d.color == 3));
}

#[test]
fn j_and_l_shuttle_and_k_stops() {
    let mut app = app();
    act(&mut app, Act::PlayForward);
    assert!(app.playing && app.play_speed == 1.0);
    act(&mut app, Act::PlayForward);
    act(&mut app, Act::PlayForward);
    assert_eq!(app.play_speed, 4.0);
    for _ in 0..6 {
        act(&mut app, Act::PlayForward);
    }
    assert_eq!(app.play_speed, MAX_SPEED);
    act(&mut app, Act::PlayReverse);
    assert_eq!(
        app.play_speed, -1.0,
        "the other direction starts again at normal speed"
    );
    act(&mut app, Act::Stop);
    assert!(!app.playing);
    act(&mut app, Act::TogglePlay);
    assert!(app.playing && app.play_speed == 1.0);
    act(&mut app, Act::TogglePlay);
    assert!(!app.playing);
}

#[test]
fn playing_in_to_out_stops_at_the_out() {
    let mut app = app();
    act(&mut app, Act::PlayInToOut);
    assert!(!app.playing && app.status.error, "no In and Out yet");
    app.run(Command::SetMarks {
        mark_in: Some(24),
        mark_out: Some(48),
    });
    act(&mut app, Act::PlayInToOut);
    assert!(app.playing);
    assert_eq!(app.playhead, 24);
    let ctx = Context::default();
    for _ in 0..200 {
        app.advance(&ctx);
    }
    assert!(!app.playing);
    assert_eq!(app.playhead, 48);
}

#[test]
fn play_around_returns_to_where_it_started() {
    let mut app = app();
    app.go_to(100);
    act(&mut app, Act::PlayAround);
    assert_eq!(app.playhead, 100 - 48);
    let ctx = Context::default();
    for _ in 0..400 {
        app.advance(&ctx);
    }
    assert!(!app.playing);
    assert_eq!(app.playhead, 100);
}

#[test]
fn looping_wraps_and_playing_backwards_stops_at_the_start() {
    let mut app = app();
    act(&mut app, Act::ToggleLoop);
    app.go_to(286);
    act(&mut app, Act::PlayForward);
    let ctx = Context::default();
    for _ in 0..20 {
        app.advance(&ctx);
    }
    assert!(
        app.playing && app.playhead < 20,
        "wrapped to the start and kept playing"
    );
    act(&mut app, Act::ToggleLoop);
    act(&mut app, Act::Stop);
    app.go_to(5);
    act(&mut app, Act::PlayReverse);
    for _ in 0..40 {
        app.advance(&ctx);
    }
    assert!(!app.playing);
    assert_eq!(app.playhead, 0);
}

#[test]
fn edit_points_and_frames() {
    let mut app = app();
    act(&mut app, Act::NextEdit);
    assert_eq!(app.playhead, 96);
    act(&mut app, Act::NextEdit);
    assert_eq!(app.playhead, 192);
    act(&mut app, Act::PreviousEdit);
    assert_eq!(app.playhead, 96);
    act(&mut app, Act::NextFrame);
    act(&mut app, Act::PreviousFrame);
    assert_eq!(app.playhead, 96);
    act(&mut app, Act::GoLast);
    assert_eq!(app.playhead, 288);
    act(&mut app, Act::NextEdit);
    assert_eq!(app.playhead, 288);
    act(&mut app, Act::SecondBack);
    assert_eq!(app.playhead, 264);
    act(&mut app, Act::GoFirst);
    assert_eq!(app.playhead, 0);
}

#[test]
fn select_forward_and_enable_and_reset_all_grades() {
    let mut app = app();
    app.go_to(100);
    act(&mut app, Act::SelectForward);
    assert_eq!(
        app.selected_ids().len(),
        2,
        "the clip under the playhead and the one after it"
    );
    let first = app.project().tracks[0].clips[0].id;
    app.select_only(first);
    app.go_to(10);
    act(&mut app, Act::ToggleEnableClip);
    assert!(!app.project().clip(first).unwrap().1.enabled);
    assert_eq!(app.act_checked(Act::ToggleEnableClip), Some(false));
    act(&mut app, Act::ToggleEnableClip);
    assert!(app.project().clip(first).unwrap().1.enabled);

    assert!(!app.act_enabled(Act::ResetAllGrades));
    for clip in app.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.id)
        .collect::<Vec<_>>()
    {
        app.run(Command::SetGrade {
            clip,
            grade: ColorGrade {
                saturation: 0.5,
                ..Default::default()
            },
            node: 0,
        });
    }
    assert!(app.act_enabled(Act::ResetAllGrades));
    act(&mut app, Act::ResetAllGrades);
    assert!(app.project().tracks[0]
        .clips
        .iter()
        .all(|c| c.grade.is_identity()));
    assert!(app.session.undo());
    assert!(
        app.project().tracks[0]
            .clips
            .iter()
            .all(|c| c.grade.saturation == 0.5),
        "one undo for all three"
    );
}

#[test]
fn generators_snap_overlays_and_pages() {
    let mut app = app();
    let media = app.project().media.len();
    for a in [Act::GenerateSolid, Act::GenerateBars, Act::GenerateGradient] {
        act(&mut app, a);
    }
    assert_eq!(app.project().media.len(), media + 3);
    assert_eq!(app.act_checked(Act::ToggleSnap), Some(true));
    act(&mut app, Act::ToggleSnap);
    assert_eq!(app.act_checked(Act::ToggleSnap), Some(false));
    assert!(!app.cut_detail.snap() && !app.cut_overview.snap());
    act(&mut app, Act::OverlayAll);
    assert!(app.overlays.safe_area && app.overlays.crosshair && app.overlays.grid);
    act(&mut app, Act::OverlayAll);
    assert!(!app.overlays.safe_area && !app.overlays.crosshair && !app.overlays.grid);
    act(&mut app, Act::PageColor);
    assert_eq!(app.page, Page::Color);
    act(&mut app, Act::PageDeliver);
    assert_eq!(app.page, Page::Deliver);
    act(&mut app, Act::FullScreenViewer);
    assert!(app.viewer_full);
    act(&mut app, Act::LeaveViewer);
    assert!(!app.viewer_full);
}

#[test]
fn revert_goes_back_to_what_was_opened() {
    let mut app = app();
    assert!(!app.act_enabled(Act::Revert));
    let text = app.project().to_json().unwrap();
    app.open_text("x.picasso", None, &text);
    assert!(app.act_enabled(Act::Revert));
    app.select_only(app.project().tracks[0].clips[0].id);
    act(&mut app, Act::DeleteSelected);
    assert_eq!(starts(&app).len(), 2);
    act(&mut app, Act::Revert);
    assert_eq!(starts(&app).len(), 3);
}

/// Makes a two-second 24 fps clip with the `ffmpeg` program; `None` when it is not installed.
#[cfg(not(target_arch = "wasm32"))]
fn video_file() -> Option<String> {
    // One file for each test (see `video_with_sound`).
    let test = std::thread::current().name().unwrap_or("test").replace("::", "_");
    let path = std::env::temp_dir().join(format!("picasso_ui_video_test_{test}.mp4"));
    std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc=duration=2:size=320x180:rate=24")
        .args(["-pix_fmt", "yuv420p"])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success())
        .then(|| path.to_string_lossy().into_owned())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_video_can_be_imported_placed_and_drawn() {
    let Some(path) = video_file() else { return };
    let mut app = app();
    app.new_project();
    app.import_file(crate::platform::ImportedFile {
        name: "clip.mp4".into(),
        path: Some(path),
        bytes: Vec::new(),
    });
    app.settle_imports();
    let item = app.project().media.last().unwrap().clone();
    assert!(matches!(item.source, MediaSource::Video { .. }));
    assert_eq!(item.length, Some(48), "two seconds at 24 fps");
    app.insert_media(item.id);
    assert_eq!(app.project().tracks[0].clips[0].duration, 48);
    // Drawing never waits for the decoder: the first picture is black and the decoded one
    // arrives in the background.
    let lit = |p: &picasso_render::Frame| p.rgba.chunks_exact(4).filter(|px| px[0] > 40).count();
    let before = app.cache.video_generation();
    let first = picasso_render::render_frame(app.project(), &app.cache, 30, 160, 90).unwrap();
    assert_eq!(lit(&first), 0, "nothing is decoded yet");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.cache.video_generation() == before && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let picture = picasso_render::render_frame(app.project(), &app.cache, 30, 160, 90).unwrap();
    assert!(lit(&picture) > 160 * 90 / 4, "the decoded picture is drawn");
    // An export waits for exact pictures.
    let exact =
        picasso_render::render_frame(app.project(), &app.cache.blocking(), 40, 160, 90).unwrap();
    assert!(
        lit(&exact) > 160 * 90 / 4,
        "the exact picture is drawn, not black"
    );
    // A file that is not a video is refused with a message, and nothing is added.
    let count = app.project().media.len();
    app.import_file(crate::platform::ImportedFile {
        name: "broken.mp4".into(),
        path: Some("no-such-file.mp4".into()),
        bytes: Vec::new(),
    });
    app.settle_imports();
    assert_eq!(app.project().media.len(), count);
    assert!(app.status.error);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn playing_a_video_never_blocks_drawing() {
    let Some(path) = video_file() else { return };
    let mut app = app();
    app.new_project();
    app.import_file(crate::platform::ImportedFile {
        name: "clip.mp4".into(),
        path: Some(path),
        bytes: Vec::new(),
    });
    app.settle_imports();
    let id = app.project().media.last().unwrap().id;
    app.insert_media(id);
    // Draw 40 frames as a player does, one every 20 ms (faster than real time).
    let (mut slowest, mut lit) = (std::time::Duration::ZERO, 0);
    for frame in 0..40 {
        let t = std::time::Instant::now();
        let p = picasso_render::render_frame(app.project(), &app.cache, frame, 160, 90).unwrap();
        slowest = slowest.max(t.elapsed());
        lit += usize::from(p.rgba.chunks_exact(4).any(|px| px[0] > 40));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        slowest < std::time::Duration::from_millis(100),
        "{slowest:?}"
    );
    assert!(lit >= 30, "most pictures were there in time: {lit} of 40");
}

/// A clip with a picture and a 440 Hz tone, for the sound tests.
#[cfg(not(target_arch = "wasm32"))]
fn video_with_sound() -> Option<String> {
    // One file for each test: they run at the same time, and one writing the file again while
    // another imports it left the other with an empty timeline.
    let test = std::thread::current().name().unwrap_or("test").replace("::", "_");
    let path = std::env::temp_dir().join(format!("picasso_ui_sound_test_{test}.mp4"));
    std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc=duration=2:size=64x64:rate=24")
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
        .args(["-pix_fmt", "yuv420p", "-shortest"])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success())
        .then(|| path.to_string_lossy().into_owned())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_video_with_sound_gets_a_waveform_and_a_play_plan() {
    let Some(path) = video_with_sound() else {
        return;
    };
    let mut app = app();
    app.new_project();
    app.open_path(&path, true);
    app.settle_imports();
    let clip = app.project().tracks[0].clips[0].clone();
    let MediaSource::Video { path } = &app.project().media[0].source else {
        panic!("not a video");
    };
    // The waveform is measured in the background and has a bucket per fiftieth of a second.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let peaks = loop {
        if let Some(p) = app.sound.peaks(clip.media, path) {
            break p;
        }
        assert!(std::time::Instant::now() < deadline, "no waveform");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!((95..=101).contains(&peaks.len()), "{}", peaks.len());
    // (AAC starts with a little silence.)
    assert!(peaks.iter().filter(|p| **p > 10).count() > 85);
    // Normalize Audio Levels: the loudest sample reaches the target.
    app.select_only(clip.id);
    let peak = app.clip_peak(clip.id).unwrap();
    app.normalize_audio(-9.0);
    let gain = app.project().clip(clip.id).unwrap().1.audio.gain_db;
    assert!((gain - (-9.0 - 20.0 * peak.log10())).abs() < 0.01, "{gain}");
    app.undo();
    // The sound is part of the plan until the clip is muted or disabled.
    assert_eq!(picasso_audio::plan(app.project()).clips.len(), 1);
    app.run(Command::SetClipEnabled {
        clip: clip.id,
        enabled: false,
    });
    assert!(picasso_audio::plan(app.project()).clips.is_empty());
}

/// A two-second WAV tone; `None` when the `ffmpeg` program is not installed.
#[cfg(not(target_arch = "wasm32"))]
fn tone_file() -> Option<String> {
    // One file for each test (see `video_with_sound`).
    let test = std::thread::current().name().unwrap_or("test").replace("::", "_");
    let path = std::env::temp_dir().join(format!("picasso_ui_tone_test_{test}.wav"));
    std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("sine=frequency=440:duration=2:sample_rate=44100")
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success())
        .then(|| path.to_string_lossy().into_owned())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_sound_file_makes_an_audio_track_and_sounds() {
    let Some(path) = tone_file() else { return };
    let mut app = app();
    app.new_project();
    // A new project starts as Resolve's does: one picture track and one sound track.
    assert_eq!(
        app.project()
            .tracks
            .iter()
            .map(|t| t.kind)
            .collect::<Vec<_>>(),
        [
            picasso_core::TrackKind::Video,
            picasso_core::TrackKind::Audio
        ]
    );
    app.open_path(&path, true);
    app.settle_imports();
    let item = app.project().media.last().unwrap().clone();
    assert!(matches!(item.source, MediaSource::Audio { .. }));
    assert_eq!(item.length, Some(48), "two seconds at 24 fps");
    // The first sound made an audio track below the video tracks and went onto it.
    let tracks = &app.project().tracks;
    assert_eq!(tracks.last().unwrap().kind, picasso_core::TrackKind::Audio);
    assert_eq!(tracks.last().unwrap().name, "A1");
    assert_eq!(tracks.last().unwrap().clips.len(), 1);
    assert!(tracks[..1].iter().all(|t| t.clips.is_empty()));
    // It is in the mix, and the picture does not change.
    assert_eq!(picasso_audio::plan(app.project()).clips.len(), 1);
    // A second one goes after it on the same track, not on a new one.
    app.go_to(48);
    app.insert_media(item.id);
    assert_eq!(
        app.project()
            .tracks
            .iter()
            .filter(|t| t.kind == picasso_core::TrackKind::Audio)
            .count(),
        1
    );
    assert_eq!(app.project().tracks.last().unwrap().clips.len(), 2);
    // A file that is not sound is refused.
    let count = app.project().media.len();
    app.open_path("no-such-file.wav", false);
    assert_eq!(app.project().media.len(), count);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_render_queue_renders_wav_and_video_one_after_another() {
    use crate::queue::native::State;
    use picasso_export::Preset;
    let Some(path) = video_with_sound() else {
        return;
    };
    let mut app = app();
    app.new_project();
    app.open_path(&path, true);
    app.settle_imports();
    let clip = app.project().tracks[0].clips[0].id;
    // Half the level, and a fade in over the first half second (12 frames at 24 fps).
    app.run(Command::SetClipAudio {
        clip,
        audio: picasso_core::ClipAudio {
            gain_db: -6.0,
            fade_in: 12,
            fade_out: 0,
        },
    });
    let dir = std::env::temp_dir();
    let (wav, mp4) = (
        dir.join("picasso_ui_queue_test.wav"),
        dir.join("picasso_ui_queue_test.mp4"),
    );
    for f in [&wav, &mp4] {
        let _ = std::fs::remove_file(f);
    }
    app.queue_render(Preset::Wav, wav.to_string_lossy().into_owned());
    app.queue_render(Preset::H264Web, mp4.to_string_lossy().into_owned());
    assert_eq!(app.deliver.states(), [State::Queued, State::Queued]);
    app.start_queue(&Context::default());
    // Editing goes on while it renders: the queue kept the project as it was.
    app.run(Command::SetClipAudio {
        clip,
        audio: picasso_core::ClipAudio::default(),
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app
        .deliver
        .states()
        .iter()
        .any(|s| matches!(s, State::Queued | State::Running))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the queue did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let states = app.deliver.states();
    assert!(
        states.iter().all(|s| matches!(s, State::Done(_))),
        "{states:?}"
    );
    // The WAV is 2 seconds of 24-bit stereo at 48 kHz after a 44-byte header, with the clip's
    // gain and fade as they were when it was queued.
    let bytes = std::fs::read(&wav).unwrap();
    assert_eq!(bytes.len(), 44 + 2 * 48_000 * 2 * 3);
    let sample = |frame: usize| {
        let i = 44 + frame * 6;
        let v = i32::from_le_bytes([0, bytes[i], bytes[i + 1], bytes[i + 2]]) >> 8;
        v as f32 / 8_388_607.0
    };
    let peak = |from: usize, to: usize| (from..to).map(|f| sample(f).abs()).fold(0.0, f32::max);
    assert!(
        peak(0, 480) < 0.01,
        "the fade in starts silent: {}",
        peak(0, 480)
    );
    let steady = peak(40_000, 48_000);
    assert!(steady > 0.03 && steady < 0.08, "the steady level: {steady}");
    assert!(
        std::fs::metadata(&mp4).is_ok_and(|m| m.len() > 1000),
        "the video exists"
    );
    // A range with nothing in it is refused and nothing is queued.
    app.new_project();
    app.queue_render(Preset::Wav, wav.to_string_lossy().into_owned());
    assert!(app.status.error);
    for f in [wav, mp4] {
        let _ = std::fs::remove_file(f);
    }
}

#[test]
fn projects_open_by_their_extensions() {
    let mut app = app();
    let text = app.project().to_json().unwrap();
    for name in ["a.picasso", "a.json"] {
        assert!(crate::platform::is_project_name(name), "{name}");
        app.open_text(name, None, &text);
        assert_eq!(app.project().tracks[0].clips.len(), 3, "{name}");
    }
    assert!(!crate::platform::is_project_name("a.mp4"));
}

#[test]
fn insert_overwrite_and_replace_edit_at_the_playhead() {
    let mut app = app();
    // The demo has three media items; select one of them and edit at frame 120.
    let media = app.project().media[0].id;
    app.selected_media = Some(media);
    app.go_to(120);
    act_edit(&mut app, Edit::Insert);
    // The second clip (96..192) was cut at 120, the new clip put in, the rest pushed by 96.
    assert_eq!(
        starts(&app),
        [(0, 96), (96, 24), (120, 96), (216, 72), (288, 96)]
    );
    // One undo step puts everything back.
    app.undo();
    assert_eq!(starts(&app), [(0, 96), (96, 96), (192, 96)]);
    app.go_to(100);
    act_edit(&mut app, Edit::Overwrite);
    assert_eq!(starts(&app), [(0, 96), (96, 4), (100, 96), (196, 92)]);
    // Replace swaps the media of the clip at the playhead and keeps its length.
    let other = app.project().media[1].id;
    app.selected_media = Some(other);
    app.go_to(10);
    act_edit(&mut app, Edit::Replace);
    assert_eq!(app.project().tracks[0].clips[0].media, other);
    assert_eq!(starts(&app)[0], (0, 96));
    // Nothing selected in the pool: a message and no change.
    app.selected_media = None;
    let before = starts(&app);
    act_edit(&mut app, Edit::Insert);
    assert_eq!(starts(&app), before);
    assert!(app.status.error);
}

enum Edit {
    Insert,
    Overwrite,
    Replace,
}

fn act_edit(app: &mut StudioApp, edit: Edit) {
    match edit {
        Edit::Insert => app.insert_edit(),
        Edit::Overwrite => app.overwrite_edit(),
        Edit::Replace => app.replace_edit(),
    }
}

/// A tiny PNG for the import tests.
fn tiny_png() -> Vec<u8> {
    let frame = picasso_render::Frame {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
    };
    picasso_render::encode_png(&frame).unwrap()
}

#[test]
fn a_saved_project_is_a_database_that_keeps_the_facts_about_its_media() {
    let mut app = app();
    app.new_project();
    app.import_file(crate::platform::ImportedFile {
        name: "pic.png".into(),
        path: None,
        bytes: tiny_png(),
    });
    let id = app.project().media[0].id;
    let text = app.project().to_json().unwrap();
    let bytes = app.project_file_bytes(&text).unwrap();
    // On the desktop the file is a SQLite database; the project text is inside.
    #[cfg(not(target_arch = "wasm32"))]
    assert!(picasso_media::is_database(&bytes));
    let mut other = StudioApp::new(&Context::default());
    other.open_bytes("p.picasso", None, bytes);
    assert_eq!(other.project(), app.project());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let facts = &other.media_facts[&id];
        assert_eq!(
            (facts.kind.as_str(), facts.width, facts.height),
            ("image", Some(2), Some(1))
        );
        assert_eq!(facts.summary(), "2 x 1");
    }
    // A project saved as JSON text (by older versions, or in the browser) still opens, without facts.
    let mut third = StudioApp::new(&Context::default());
    third.open_bytes("old.json", None, text.into_bytes());
    assert_eq!(third.project(), app.project());
    assert!(third.media_facts.is_empty());
    // Something that is neither is refused with a message and nothing changes.
    let before = third.project().clone();
    third.open_bytes("junk.picasso", None, vec![0xFF, 0xFE, 0x00, 0x12]);
    assert!(third.status.error);
    assert_eq!(*third.project(), before);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn facts_about_imported_video_and_sound_are_kept() {
    let Some(path) = video_with_sound() else {
        return;
    };
    let mut app = app();
    app.new_project();
    app.open_path(&path, false);
    app.settle_imports();
    let id = app.project().media[0].id;
    let facts = &app.media_facts[&id];
    assert_eq!(facts.kind, "video");
    assert_eq!((facts.width, facts.height), (Some(64), Some(64)));
    assert_eq!(facts.audio_codec.as_deref(), Some("aac"));
    // Facts about media that was removed are not saved.
    app.run(Command::RemoveMedia { media: id });
    let bytes = app.project_file_bytes("{}").unwrap();
    let (_, kept) = picasso_media::from_bytes(&bytes).unwrap();
    assert!(kept.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
mod deliver_page {
    use super::*;
    use picasso_export::{Preset, Quality, Rate};

    fn names(app: &StudioApp) -> Vec<String> {
        app.jobs().into_iter().map(|j| j.name).collect()
    }

    #[test]
    fn the_form_sets_the_size_frame_rate_and_key_frames_of_the_render() {
        let mut app = app();
        let o = app.export_options();
        // A render starts at 60 frames per second.
        assert_eq!(
            (o.size, o.fps, o.keyframes),
            (None, Some(picasso_core::Fps::FPS_60), None)
        );
        app.deliver_ui.res_mode = 4; // 4K UHD
        app.deliver_ui.fps_choice = 3; // the third preset: 30
        app.deliver_ui.key_auto = false;
        app.deliver_ui.every = 12;
        let o = app.export_options();
        assert_eq!(o.size, Some((3840, 2160)));
        assert_eq!(o.fps, picasso_core::Fps::PRESETS.get(2).copied());
        assert_eq!(o.keyframes, Some(12));
        // A custom size, and a custom vertical resolution that keeps the timeline's shape.
        app.deliver_ui.res_mode = crate::deliver::RESOLUTIONS.len() + 1;
        app.deliver_ui.custom = (800, 450);
        assert_eq!(app.export_options().size, Some((800, 450)));
        app.deliver_ui.vertical = true;
        assert_eq!(app.export_options().size, Some((800, 450)));
        app.deliver_ui.custom = (0, 720);
        assert_eq!(app.export_options().size, Some((1280, 720)));
    }

    #[test]
    fn the_advanced_settings_reach_the_export_options() {
        use picasso_export::{BurnIn, Levels, PixelAspect};
        let mut app = app();
        let o = app.export_options();
        assert_eq!(o.pixel_aspect, PixelAspect::Square);
        assert_eq!((o.levels, o.burn_in), (Levels::Auto, BurnIn::Off));
        assert_eq!((o.color_primaries, o.color_trc), (None, None));
        app.deliver_ui.cinemascope = true;
        app.deliver_ui.levels = 2;
        app.deliver_ui.color_tag = 5; // P3-D65
        app.deliver_ui.gamma_tag = 4; // sRGB
        app.deliver_ui.burn = 4;
        let o = app.export_options();
        assert_eq!(o.pixel_aspect, PixelAspect::Cinemascope);
        assert_eq!(o.levels, Levels::Full);
        assert_eq!(o.color_primaries, Some("smpte432"));
        assert_eq!(o.color_trc, Some("iec61966-2-1"));
        assert_eq!(o.burn_in, BurnIn::Both);
        // The page's clock starts an hour in, as its timecodes do.
        assert_eq!(o.clock_start, 3600 * 24);
        // A choice past the end of a list is the project's, not a crash.
        app.deliver_ui.color_tag = 99;
        app.deliver_ui.burn = 99;
        let o = app.export_options();
        assert_eq!((o.color_primaries, o.burn_in), (None, BurnIn::Off));
    }

    #[test]
    fn the_estimate_follows_the_settings() {
        let mut app = app();
        let h264 = app.estimated_mb();
        assert!(h264 > 0.0);
        app.deliver.options.rate = Rate::Quality(Quality::Low);
        assert!(app.estimated_mb() < h264);
        app.deliver.options.rate = Rate::Quality(Quality::Best);
        app.deliver.preset = Preset::ProRes422;
        assert!(app.estimated_mb() > h264);
        app.deliver.options.video = false;
        app.deliver.preset = Preset::H264Web;
        assert!(app.estimated_mb() < h264, "no picture, a smaller file");
        app.new_project();
        assert_eq!(app.estimated_mb(), 0.0);
    }

    #[test]
    fn a_location_and_name_queue_a_render_without_asking_and_unique_names_do_not_collide() {
        let mut app = app();
        let dir = std::env::temp_dir().join("picasso_deliver_page_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.deliver_ui.location = dir.to_string_lossy().into_owned();
        app.deliver_ui.file_name = "cut".into();
        app.deliver.preset = Preset::H264Web;
        let ctx = Context::default();
        app.add_to_queue(&ctx);
        assert_eq!(names(&app), ["cut.mp4"]);
        // Without "unique" the same name is queued again; with it, the next free name.
        app.deliver_ui.unique = true;
        app.add_to_queue(&ctx);
        app.add_to_queue(&ctx);
        assert_eq!(names(&app), ["cut.mp4", "cut (1).mp4", "cut (2).mp4"]);
        // A folder chosen with Browse is the location and queues nothing.
        app.deliver_ui.browsing = true;
        app.enqueue_export("somewhere".into());
        assert_eq!(app.deliver_ui.location, "somewhere");
        assert_eq!(names(&app).len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn individual_clips_are_a_render_each_and_need_a_location() {
        let mut app = app();
        app.deliver_ui.individual = true;
        app.deliver_ui.file_name = "shot".into();
        let ctx = Context::default();
        app.add_to_queue(&ctx);
        assert!(app.status.error, "no location, nothing queued");
        assert!(names(&app).is_empty());
        app.deliver_ui.location = std::env::temp_dir().to_string_lossy().into_owned();
        app.add_to_queue(&ctx);
        assert_eq!(names(&app), ["shot_01.mp4", "shot_02.mp4", "shot_03.mp4"]);
    }

    #[test]
    fn the_range_choice_is_kept_and_jobs_can_be_removed() {
        let mut app = app();
        assert!(app.range_is_whole());
        app.set_range_whole(false);
        assert!(!app.range_is_whole());
        app.deliver_ui.location = std::env::temp_dir().to_string_lossy().into_owned();
        app.deliver_ui.file_name = "range".into();
        app.add_to_queue(&Context::default());
        let id = app.jobs()[0].id;
        app.cancel_job(id);
        assert_eq!(app.jobs()[0].state, crate::queue::JobState::Cancelled);
        app.clear_finished_jobs();
        assert!(app.jobs().is_empty());
    }
}

#[test]
fn proxy_mode_shrinks_the_viewer_and_is_checked_in_the_menu() {
    let mut app = app();
    assert_eq!(app.act_checked(Act::ProxyOff), Some(true));
    act(&mut app, Act::ProxyQuarter);
    assert_eq!(app.proxy, 4);
    assert_eq!(app.act_checked(Act::ProxyQuarter), Some(true));
    assert_eq!(app.act_checked(Act::ProxyOff), Some(false));
    act(&mut app, Act::ProxyOff);
    assert_eq!(app.proxy, 1);
}

/// The app with a video on V1 at 0 for 96 frames, placed with its linked sound on A1.
fn app_with_linked_video() -> (StudioApp, ClipId, ClipId) {
    let mut app = app();
    let Some(Outcome::Media(m)) = app.run(Command::AddMedia {
        name: "v".into(),
        source: picasso_core::MediaSource::Video {
            path: "missing.mp4".into(),
        },
        length: Some(1000),
        default_duration: 96,
    }) else {
        panic!()
    };
    let v2 = app.project().tracks[1].id;
    let Some(Outcome::Clip(picture)) = app.run(Command::AddClip {
        track: v2,
        media: m,
        start: 0,
        duration: None,
        with_sound: true,
    }) else {
        panic!()
    };
    let sound = app.project().linked_to(picture)[0];
    (app, picture, sound)
}

#[test]
fn selecting_a_linked_clip_selects_its_sound_and_delete_takes_both_once() {
    let (mut app, picture, sound) = app_with_linked_video();
    app.select_only(picture);
    assert!(
        app.is_selected(sound),
        "the linked sound is selected with it"
    );
    assert_eq!(app.selected_ids().len(), 2);
    act(&mut app, Act::DeleteSelected);
    assert!(app.project().clip(picture).is_none());
    assert!(app.project().clip(sound).is_none());
    assert!(
        app.status.text.is_empty() || !app.status.error,
        "{}",
        app.status.text
    );
}

#[test]
fn linked_selection_toggles_and_lets_one_side_be_selected_alone() {
    let (mut app, picture, sound) = app_with_linked_video();
    assert_eq!(app.act_checked(Act::ToggleLinkedSelection), Some(true));
    act(&mut app, Act::ToggleLinkedSelection);
    assert_eq!(app.act_checked(Act::ToggleLinkedSelection), Some(false));
    app.select_only(sound);
    assert!(!app.is_selected(picture));
    act(&mut app, Act::DeleteSelected);
    assert!(app.project().clip(picture).is_some(), "only the sound went");
}

#[test]
fn copy_and_paste_keeps_the_pair_linked_and_heard_once() {
    let (mut app, picture, _) = app_with_linked_video();
    app.select_only(picture);
    act(&mut app, Act::Copy);
    app.go_to(200);
    act(&mut app, Act::Paste);
    let pasted = app
        .project()
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.start == 200)
        .map(|c| (c.id, c.sound))
        .collect::<Vec<_>>();
    assert_eq!(pasted.len(), 2);
    let (a, b) = (pasted[0].0, pasted[1].0);
    assert_eq!(app.project().linked_to(a), vec![b]);
    assert_eq!(
        pasted.iter().filter(|(_, sound)| *sound).count(),
        1,
        "one of the two is heard"
    );
    // One undo step takes the whole paste back.
    app.session.undo();
    assert!(app.project().clip(a).is_none() && app.project().clip(b).is_none());
}

#[test]
fn link_and_unlink_from_the_clip_menu() {
    let (mut app, picture, sound) = app_with_linked_video();
    app.select_only(picture);
    assert!(app.act_enabled(Act::UnlinkClips));
    act(&mut app, Act::UnlinkClips);
    assert!(app.project().linked_to(picture).is_empty());
    app.select_many(vec![picture, sound]);
    assert!(app.act_enabled(Act::LinkClips));
    act(&mut app, Act::LinkClips);
    assert_eq!(app.project().linked_to(picture), vec![sound]);
}

#[test]
fn the_color_page_edits_the_chosen_node() {
    let mut app = app();
    let clip = app.project().tracks[0].clips[0].id;
    app.select_only(clip);
    app.go_to(0);
    act(&mut app, Act::AddSerialNode);
    assert_eq!(app.color_node, 1, "the new node is chosen");
    assert_eq!(app.node_for(clip), 1);
    act(&mut app, Act::ToggleNode);
    let c = app.project().clip(clip).unwrap().1;
    assert_eq!(c.node_count(), 2);
    assert!(!c.nodes[0].enabled, "Ctrl+D switched node 02 off");
    assert!(c.grade.enabled);
    // A clip with fewer nodes edits its last one.
    let other = app.project().tracks[0].clips[1].id;
    assert_eq!(app.node_for(other), 0);
    act(&mut app, Act::ResetAllGrades);
    assert_eq!(app.project().clip(clip).unwrap().1.node_count(), 1);
}

#[test]
fn cutting_linked_clips_at_the_playhead_cuts_each_once() {
    let mut app = app();
    let a = app.project().tracks[0].clips[0].clone();
    let Some(Outcome::Clip(top)) = app.run(Command::PlaceOnTop {
        media: a.media,
        start: a.start,
        with_sound: false,
    }) else {
        panic!("placed on top");
    };
    app.run(Command::LinkClips {
        clips: vec![a.id, top],
    });
    app.selection = None;
    app.go_to(a.start + 40);
    act(&mut app, Act::SplitClip);
    assert!(!app.status.error, "{}", app.status.text);
    let cut_at = |app: &StudioApp| {
        app.project()
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.start == a.start + 40)
            .count()
    };
    assert_eq!(cut_at(&app), 2, "both linked clips are cut");
}

#[test]
fn a_window_grades_part_of_the_picture_and_highlight_shows_its_key() {
    let mut app = app();
    let a = app.project().tracks[0].clips[0].clone();
    app.go_to(a.start + 2);
    let grade = picasso_color::ColorGrade {
        gain: [2.0; 3],
        windows: vec![picasso_color::PowerWindow::new(
            picasso_color::WindowShape::Circle,
        )],
        ..Default::default()
    };
    app.run(Command::SetGrade {
        clip: a.id,
        grade,
        node: 0,
    });
    assert!(app.project().clip(a.id).unwrap().1.grade.is_secondary());
    app.page = crate::app::Page::Color;
    app.color.highlight = true;
    let stamp = (app.epoch, app.session.revision(), app.media_generation);
    app.update_highlight(stamp);
    let (_, project, _) = app.highlighted.as_ref().unwrap();
    assert!(project.clip(a.id).unwrap().1.grade.highlight);
    assert!(
        !app.project().clip(a.id).unwrap().1.grade.highlight,
        "only on the copy"
    );
    app.color.highlight = false;
    app.update_highlight(stamp);
    assert!(app.highlighted.is_none());
}

#[test]
fn the_new_menu_entries_do_what_they_say() {
    let mut app = app();
    let [a, b, _] = [0, 1, 2].map(|i| app.project().tracks[0].clips[i].clone());
    // Mark Timeline In/Out.
    act(&mut app, Act::MarkTimeline);
    assert_eq!(
        (app.project().mark_in, app.project().mark_out),
        (Some(0), Some(app.project().duration() - 1))
    );
    // Select Nearest Edit Point goes to the cut and selects the clip after it.
    app.go_to(b.start + 3);
    act(&mut app, Act::SelectNearestEdit);
    assert_eq!((app.playhead, app.selection), (b.start, Some(b.id)));
    // Next and Previous Node Graph step clip by clip.
    app.go_to(a.start + 5);
    act(&mut app, Act::NextNodeGraph);
    assert_eq!(app.playhead, b.start);
    act(&mut app, Act::PreviousNodeGraph);
    assert_eq!(app.playhead, a.start);
    // Add Serial Before keeps the grade on the node after the new one.
    let warm = picasso_color::ColorGrade {
        saturation: 1.5,
        ..Default::default()
    };
    app.run(Command::SetGrade {
        clip: a.id,
        grade: warm,
        node: 0,
    });
    act(&mut app, Act::AddNodeBefore);
    let clip = app.project().clip(a.id).unwrap().1.clone();
    assert_eq!(clip.node_count(), 2);
    assert_eq!(clip.grade.saturation, 1.0);
    assert_eq!(clip.node(1).unwrap().saturation, 1.5);
    // Change Clip Speed opens its window for the clip.
    app.select_only(a.id);
    act(&mut app, Act::ChangeClipSpeed);
    assert_eq!(app.extra.speed, Some((a.id, 100.0)));
    // Toggles.
    act(&mut app, Act::PositionLock);
    assert_eq!(app.act_checked(Act::PositionLock), Some(true));
    act(&mut app, Act::SelectionFollowsPlayhead);
    app.go_to(b.start + 1);
    app.follow_playhead();
    assert_eq!(app.selection, Some(b.id));
    // New Timeline Using Selected Clips.
    let before = app.project().timeline_list().len();
    app.select_only(b.id);
    act(&mut app, Act::TimelineFromSelection);
    assert_eq!(app.project().timeline_list().len(), before + 1);
    assert_eq!(app.project().tracks[0].clips.len(), 1);
    assert_eq!(app.project().tracks[0].clips[0].start, 0);
}

#[test]
fn cutting_at_markers_is_one_step() {
    let mut app = app();
    for frame in [20, 50, 96, 130] {
        app.run(Command::SetMarker {
            frame,
            name: String::new(),
            color: 0,
        });
    }
    let clips = app.project().tracks[0].clips.len();
    app.cut_at_markers();
    assert!(!app.status.error, "{}", app.status.text);
    // 96 is a cut already; the other three fall inside clips.
    assert_eq!(app.project().tracks[0].clips.len(), clips + 3);
    app.undo();
    assert_eq!(app.project().tracks[0].clips.len(), clips);
}

#[test]
fn clips_lock_take_colours_and_normalize() {
    let mut app = app();
    let a = app.project().tracks[0].clips[0].clone();
    app.select_only(a.id);
    act(&mut app, Act::LockClip);
    assert!(app.project().clip(a.id).unwrap().1.locked);
    assert_eq!(
        app.session.dispatch(Command::Delete { clip: a.id }),
        Err(picasso_core::EditError::ClipLocked)
    );
    act(&mut app, Act::LockClip);
    assert!(!app.project().clip(a.id).unwrap().1.locked);
    act(&mut app, Act::SetClipColor(Some(8)));
    assert_eq!(app.project().clip(a.id).unwrap().1.color, Some(8));
    assert_eq!(app.act_checked(Act::SetClipColor(Some(8))), Some(true));
    act(&mut app, Act::SetClipColor(None));
    assert_eq!(app.project().clip(a.id).unwrap().1.color, None);
    // The demo's clips are generated pictures: nothing to measure.
    app.normalize_audio(-9.0);
    assert!(app.status.error);
}

#[test]
fn the_pool_searches_and_sorts() {
    let mut app = app();
    let names: Vec<String> = app.project().media.iter().map(|m| m.name.clone()).collect();
    assert!(names.len() >= 2);
    app.pool_tool("Search");
    assert!(app.pool.searching);
    app.pool.search = names[1].to_uppercase();
    let found = app.pool_items();
    assert!(found
        .iter()
        .all(|m| m.name.to_lowercase().contains(&names[1].to_lowercase())));
    assert!(!found.is_empty());
    app.pool_tool("Search");
    assert!(app.pool.search.is_empty());
    app.pool_tool("Sort");
    let sorted: Vec<String> = app
        .pool_items()
        .into_iter()
        .map(|m| m.name.to_lowercase())
        .collect();
    let mut expected = sorted.clone();
    expected.sort();
    assert_eq!(sorted, expected);
    assert!(app.pool_tool("List"));
    assert_eq!(app.pool.view, crate::pool::PoolView::List);
    assert!(!app.pool_tool("Something else"));
}

#[test]
fn an_edl_comes_back_as_a_new_timeline() {
    let mut app = app();
    let original: Vec<(u64, u64, u64, picasso_core::MediaId)> = app.project().tracks[0]
        .clips
        .iter()
        .map(|c| (c.start, c.duration, c.source_in, c.media))
        .collect();
    let text = picasso_core::edl::to_edl(app.project(), "Cut");
    app.open_bytes("Cut v2.edl", None, text.into_bytes());
    assert!(!app.status.error, "{}", app.status.text);
    assert_eq!(app.project().timeline_name, "Cut v2");
    let video = app
        .project()
        .tracks
        .iter()
        .find(|t| t.kind == picasso_core::TrackKind::Video)
        .unwrap();
    let back: Vec<_> = video
        .clips
        .iter()
        .map(|c| (c.start, c.duration, c.source_in, c.media))
        .collect();
    assert_eq!(back, original);
    // A list whose media is not in the pool says so.
    app.open_bytes(
        "x.edl",
        None,
        b"001  AX V C 00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00\n* FROM CLIP NAME: nope.mov\n"
            .to_vec(),
    );
    assert!(app.status.error);
}

#[test]
fn subtitles_come_in_as_titles_and_go_out_as_srt() {
    let mut app = app();
    let tracks = app.project().tracks.len();
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,500\nWorld\nagain\n";
    app.open_bytes("film.srt", None, srt.as_bytes().to_vec());
    assert!(!app.status.error, "{}", app.status.text);
    assert_eq!(app.project().tracks.len(), tracks + 1);
    let fps = app.project().fps;
    let cues = crate::subtitles::cues(app.project(), 0, app.project().duration());
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[1].text, "World\nagain");
    assert_eq!(cues[0].start, fps.nominal());
    // One undo takes them all back.
    app.undo();
    assert_eq!(app.project().tracks.len(), tracks);
    // A region at the playhead.
    app.go_to(10);
    act(&mut app, Act::CreateSubtitleRegion);
    let cues = crate::subtitles::cues(app.project(), 0, app.project().duration());
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].start, 10);
}

#[test]
fn a_still_of_the_timeline_joins_the_pool() {
    let mut app = app();
    let before = app.project().media.len();
    app.go_to(5);
    act(&mut app, Act::StillToPool);
    assert!(!app.status.error, "{}", app.status.text);
    assert_eq!(app.project().media.len(), before + 1);
    let still = app.project().media.last().unwrap();
    assert!(
        still.name.starts_with("Still 01_00_00_05"),
        "{}",
        still.name
    );
    assert!(matches!(still.source, MediaSource::Image { .. }));
}

#[test]
fn bins_show_their_media() {
    use crate::pool::BinView;
    let mut app = app();
    let all = app.pool_items().len();
    let first = app.project().media[0].id;
    app.new_bin();
    let BinView::Bin(bin) = app.pool.bin else {
        panic!("the new bin is shown");
    };
    assert!(app.pool_items().is_empty());
    assert_eq!(app.pool_bin_title(), "Bin 1");
    app.run(Command::SetMediaBin {
        media: first,
        bin: Some(bin),
    });
    assert_eq!(app.pool_items().len(), 1);
    app.pool.bin = BinView::Master;
    assert_eq!(app.pool_items().len(), all, "Master shows everything");
    app.run(Command::AddSmartBin {
        name: "Named".into(),
        contains: app.project().media[0].name.clone(),
        kind: None,
    });
    app.next_bin();
    app.next_bin();
    assert!(matches!(app.pool.bin, BinView::Smart(_)));
    assert!(!app.pool_items().is_empty());
    app.next_bin();
    assert_eq!(app.pool.bin, BinView::Master);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_folder_comes_in_as_a_bin() {
    let mut app = app();
    let dir = std::env::temp_dir().join("picasso_folder_bin_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let frame = picasso_render::render_frame(app.project(), &app.cache, 0, 16, 16).unwrap();
    let png = picasso_render::encode_png(&frame).unwrap();
    for name in ["a.png", "b.png"] {
        std::fs::write(dir.join(name), &png).unwrap();
    }
    std::fs::write(dir.join("notes.txt"), "not media").unwrap();
    app.import_folder_as_bin(dir.to_str().unwrap());
    assert!(!app.status.error, "{}", app.status.text);
    assert_eq!(app.pool_bin_title(), "picasso_folder_bin_test");
    assert_eq!(app.pool_items().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn match_frame_goes_both_ways() {
    let mut app = app();
    let b = app.project().tracks[0].clips[1].clone();
    app.go_to(b.start + 10);
    act(&mut app, Act::MatchFrame);
    assert_eq!(app.source.media, Some(b.media));
    assert_eq!(app.source.frame, b.source_frame_at(b.start + 10));
    // Back from the source to the timeline.
    app.go_to(0);
    app.match_frame_to_timeline();
    assert_eq!(app.playhead, b.start + 10);
    assert_eq!(app.selection, Some(b.id));
    // The source plays on its own.
    app.source_go_to(0);
    app.source_toggle_play();
    assert!(app.source.playing);
    app.source_go_to(u64::MAX);
    assert_eq!(app.source.frame, app.source_length() - 1);
    assert!(!app.source.playing);
}

#[test]
fn trim_mode_ripples_both_ways() {
    let mut app = app();
    act(&mut app, Act::TrimMode);
    assert_eq!(app.act_checked(Act::TrimMode), Some(true));
    let a = app.project().tracks[0].clips[0].clone();
    // Longer at the end: the clips after it move right.
    app.ripple_edit(a.id, false, a.end() + 10);
    assert_eq!(starts(&app), vec![(0, 106), (106, 96), (202, 96)]);
    // Shorter at the start: the clip stays at 0, the rest close up.
    app.ripple_edit(a.id, true, 20);
    assert_eq!(starts(&app), vec![(0, 86), (86, 96), (182, 96)]);
    assert_eq!(app.project().tracks[0].clips[0].source_in, a.source_in + 20);
    // Slipping moves the footage, not the clip.
    app.slip_by(a.id, 5);
    assert_eq!(app.project().tracks[0].clips[0].source_in, a.source_in + 15);
    assert_eq!(starts(&app)[0], (0, 86));
    act(&mut app, Act::SelectionMode);
    assert!(!app.extra.trim_mode);
}

#[test]
fn a_speed_ramp_from_the_clip_menu() {
    let mut app = app();
    let a = app.project().tracks[0].clips[0].clone();
    app.select_only(a.id);
    act(&mut app, Act::SpeedRamp);
    let dialog = app.extra.ramp.clone().unwrap();
    let speeds = dialog.speeds();
    assert_eq!(speeds.len(), 8);
    assert!(speeds.windows(2).all(|w| w[1] > w[0]), "faster and faster");
    assert!(speeds[0] > 1.0 && speeds[7] < 3.0);
    app.run(Command::SpeedRamp { clip: a.id, speeds });
    assert_eq!(app.project().tracks[0].clips.len(), 3 - 1 + 8);
    app.undo();
    assert_eq!(app.project().tracks[0].clips.len(), 3);
}

#[test]
fn animated_values_are_written_as_keyframes() {
    use picasso_core::KeyParam;
    let ctx = Context::default();
    let mut app = app();
    let a = app.project().tracks[0].clips[0].clone();
    // Not animated: the clip's own value.
    app.go_to(a.start + 10);
    assert!(!app.set_animated(&ctx, a.id, KeyParam::Opacity, 0.3));
    // A keyframe at 10, then a change at 50 adds one there.
    app.run(Command::SetKeyframe {
        clip: a.id,
        param: KeyParam::Opacity,
        at: 10,
        value: Some(1.0),
        ease: false,
    });
    app.go_to(a.start + 50);
    assert!(app.set_animated(&ctx, a.id, KeyParam::Opacity, 0.0));
    let clip = app.project().clip(a.id).unwrap().1.clone();
    assert_eq!(clip.keys(KeyParam::Opacity).unwrap().len(), 2);
    assert_eq!(clip.opacity_at(a.start + 30), 0.5);
    let (opacity, _) = app.shown_transform(a.id);
    assert_eq!(opacity, 0.0);
    assert_eq!(app.clip_frame(a.id), Some(50));
}

#[test]
fn text_plus_and_effects_from_the_menus() {
    use picasso_core::EffectKind;
    let mut app = app();
    app.go_to(10);
    act(&mut app, Act::AddTextPlus);
    let id = app.selection.unwrap();
    let (_, clip) = app.project().clip(id).unwrap();
    let media = app.project().media(clip.media).unwrap();
    let MediaSource::Title(t) = &media.source else {
        panic!("a title");
    };
    assert!(t.style.outline > 0 && t.style.shadow > 0);
    // An effect on the clip, then its strength, then away.
    let a = app.project().tracks[0].clips[0].id;
    app.select_only(a);
    act(&mut app, Act::AddEffect(EffectKind::Vignette));
    let e = app.project().clip(a).unwrap().1.effects[0];
    assert_eq!(e.kind, EffectKind::Vignette);
    app.run(Command::SetEffect {
        clip: a,
        index: 0,
        effect: picasso_core::Effect { amount: 500.0, ..e },
    });
    assert_eq!(app.project().clip(a).unwrap().1.effects[0].amount, 100.0);
    app.run(Command::RemoveEffect { clip: a, index: 0 });
    assert!(app.project().clip(a).unwrap().1.effects.is_empty());
    // A mask from the Effects page lands on the Color page's Window palette.
    app.go_to(5);
    app.add_window_from_effects(picasso_color::WindowShape::Circle);
    assert_eq!(app.page, crate::app::Page::Color);
    assert!(app.project().clip(a).unwrap().1.grade.is_secondary());
}

#[test]
fn track_eq_and_dynamics_from_the_menus() {
    let mut app = app();
    // The demo has no sound track yet: the menu says so.
    let has_sound = app
        .project()
        .tracks
        .iter()
        .any(|t| t.kind == picasso_core::TrackKind::Audio);
    if !has_sound {
        app.run(Command::AddTrack {
            kind: picasso_core::TrackKind::Audio,
        });
    }
    act(&mut app, Act::TrackDynamics);
    let (track, tab) = app.extra.track_fx.unwrap();
    assert_eq!(tab, crate::track_fx::FxTab::Dynamics);
    let mut fx = app.track_fx_of(track);
    fx.dynamics.enabled = true;
    fx.eq.enabled = true;
    fx.eq.mid_gain = 6.0;
    app.run(Command::SetTrackFx { track, fx });
    let plan = picasso_audio::plan(app.project());
    let index = app
        .project()
        .tracks
        .iter()
        .position(|t| t.id == track)
        .unwrap();
    assert!(!plan.tracks[index].is_plain());
    let back = picasso_core::Project::from_json(&app.project().to_json().unwrap()).unwrap();
    assert_eq!(back.track(track).unwrap().fx, fx);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_recorded_take_lands_on_the_armed_track() {
    let mut app = app();
    let Some(Outcome::Track(track)) = app.run(Command::AddTrack {
        kind: picasso_core::TrackKind::Audio,
    }) else {
        panic!("a sound track");
    };
    // Nothing armed: the record button says so.
    app.toggle_recording();
    assert!(app.status.error);
    // Half a second of a tone, as the device would give it.
    let samples: Vec<f32> = (0..24_000)
        .flat_map(|i| {
            let v = 0.3 * (i as f32 * 0.05).sin();
            [v, v]
        })
        .collect();
    let media = app.keep_take(&samples, 48_000, 24, &[track]).unwrap();
    assert!(!app.status.error, "{}", app.status.text);
    let t = app.project().track(track).unwrap();
    assert_eq!(t.clips.len(), 1);
    assert_eq!((t.clips[0].start, t.clips[0].media), (24, media));
    let fps = app.project().fps.nominal();
    assert!(
        t.clips[0].duration.abs_diff(fps / 2) <= 1,
        "{}",
        t.clips[0].duration
    );
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("picasso_recordings_test"));
}

#[test]
fn parallel_and_layer_nodes_join_the_current_node() {
    let mut app = app();
    let clip = app.project().tracks[0].clips[0].id;
    app.select_only(clip);
    app.go_to(0);
    act(&mut app, Act::AddParallelNode);
    act(&mut app, Act::AddLayerNode);
    let c = app.project().clip(clip).unwrap().1;
    assert_eq!(c.node_count(), 3);
    assert_eq!(c.nodes[0].mode, picasso_color::NodeMode::Parallel);
    assert_eq!(c.nodes[1].mode, picasso_color::NodeMode::Layer);
    assert_eq!(app.color_node, 2);
}

#[test]
fn the_multicam_viewer_opens_and_a_new_clip_needs_two_videos() {
    let mut app = app();
    act(&mut app, Act::MulticamViewer);
    assert!(app.multicam.viewer);
    act(&mut app, Act::MulticamViewer);
    assert!(!app.multicam.viewer);
    // The demo's media are generators, so there is nothing to make a multicam clip of.
    act(&mut app, Act::NewMulticam);
    assert!(app.multicam.creating.is_none());
}

#[test]
fn an_outside_node_takes_the_inverse_of_the_current_nodes_key() {
    let mut app = app();
    let clip = app.project().tracks[0].clips[0].id;
    app.select_only(clip);
    app.go_to(0);
    // Without a key there is nothing to invert.
    act(&mut app, Act::AddOutsideNode);
    assert_eq!(app.project().clip(clip).unwrap().1.node_count(), 1);
    let grade = picasso_color::ColorGrade {
        windows: vec![picasso_color::PowerWindow::default()],
        ..Default::default()
    };
    app.run(Command::SetGrade {
        clip,
        grade,
        node: 0,
    });
    act(&mut app, Act::AddOutsideNode);
    let c = app.project().clip(clip).unwrap().1;
    assert_eq!(c.node_count(), 2);
    assert_ne!(c.grade.windows[0].invert, c.nodes[0].windows[0].invert);
}

#[test]
fn a_timeline_can_be_made_from_the_pool() {
    let mut app = app();
    let media = app.project().media.len();
    act(&mut app, Act::TimelineFromBin);
    assert_eq!(app.project().timelines.len(), 1, "the old timeline is kept");
    let clips: usize = app.project().tracks.iter().map(|t| t.clips.len()).sum();
    assert_eq!(clips, media);
}

#[test]
fn dynamic_trim_rolls_the_nearest_edit_to_the_playhead() {
    let mut app = app();
    let (first, second) = (
        app.project().tracks[0].clips[0].id,
        app.project().tracks[0].clips[1].id,
    );
    app.go_to(100);
    act(&mut app, Act::DynamicTrim);
    assert!(app.extra.dynamic_trim);
    assert_eq!(app.extra.dynamic_edit, Some((first, second)));
    app.go_to(110);
    app.dynamic_trim_apply();
    let c = app.project().clip(second).unwrap().1;
    assert_eq!(c.start, 110);
    assert_eq!(app.project().clip(first).unwrap().1.end(), 110);
    act(&mut app, Act::DynamicTrim);
    assert!(!app.extra.dynamic_trim);
}

#[test]
fn trim_options_toggle() {
    let mut app = app();
    act(&mut app, Act::LoopTrim);
    act(&mut app, Act::AsymmetricTrim);
    assert!(app.extra.loop_trim && app.extra.asymmetric);
    assert_eq!(app.act_checked(Act::LoopTrim), Some(true));
}

#[test]
fn clip_attributes_rename_the_clip() {
    let mut app = app();
    let clip = app.project().tracks[0].clips[0].id;
    app.select_only(clip);
    act(&mut app, Act::ClipAttributes);
    assert_eq!(app.extra.attributes.as_ref().map(|a| a.0), Some(clip));
    app.run(Command::RenameClip {
        clip,
        name: "Opening".into(),
    });
    assert_eq!(app.project().clip(clip).unwrap().1.name, "Opening");
}

#[test]
fn render_in_place_bakes_the_grade_into_a_new_file() {
    let mut app = app();
    let clip = app.project().tracks[0].clips[0].id;
    app.select_only(clip);
    app.run(Command::SetGrade {
        clip,
        grade: picasso_color::ColorGrade {
            gain: [2.0, 1.0, 1.0],
            ..Default::default()
        },
        node: 0,
    });
    let media = app.project().media.len();
    act(&mut app, Act::RenderInPlace);
    // The render runs in the background; its result arrives through the inbox.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.project().media.len() == media && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(100));
        app.process_inbox();
    }
    let c = app.project().clip(clip).unwrap().1;
    assert_eq!(
        app.project().media.len(),
        media + 1,
        "{:?}",
        app.status.text
    );
    assert!(!c.is_graded(), "the grade is baked in: {}", app.status.text);
    assert_eq!(c.media, app.project().media.last().unwrap().id);
}

#[test]
fn resetting_the_layout_puts_every_panel_back() {
    let mut app = app();
    app.extra.prefs.panels.set_edit_column(1, 700.0);
    app.extra.prefs.panels.cut_upper = 400;
    app.extra.prefs.panels.moved.insert("color gallery".into(), -35);
    act(&mut app, Act::ResetLayout);
    assert_eq!(app.extra.prefs.panels, crate::panels::Panels::default());
}

#[test]
fn the_layout_entries_open_their_windows() {
    let mut app = app();
    act(&mut app, Act::SaveLayout);
    assert_eq!(app.extra.save_layout, Some(String::new()));
    assert!(!app.extra.load_layout);
    act(&mut app, Act::LoadLayout);
    assert!(app.extra.load_layout);
}

/// Waits (up to ten seconds) until `done` says so, taking in what the background threads sent.
fn wait_for(app: &mut StudioApp, done: impl Fn(&StudioApp) -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        app.process_inbox();
        if done(app) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    false
}

#[test]
fn a_resolve_file_is_read_in_the_background() {
    let mut app = app();
    let before = app.project().name.clone();
    let xml = r#"<?xml version="1.0"?><xmeml><sequence><name>Cut 1</name>
        <rate><timebase>25</timebase></rate>
        <file><pathurl>file:///nowhere/shot.mov</pathurl></file></sequence></xmeml>"#;
    app.open_bytes("cut.xml", None, xml.as_bytes().to_vec());
    // Nothing has been read when the call returns: the file is read on another thread.
    assert_eq!(app.project().name, before);
    assert!(
        wait_for(&mut app, |a| a.project().name == "Cut 1"),
        "the project never arrived: {}",
        app.status.text
    );
    // The clip is not on this computer, so the pool lists it offline.
    assert_eq!(app.project().media.len(), 1);
}

#[test]
fn a_damaged_resolve_file_says_so_and_changes_nothing() {
    let mut app = app();
    let before = app.project().name.clone();
    app.open_bytes("bad.drp", None, b"this is not an archive".to_vec());
    assert!(
        wait_for(&mut app, |a| a.status.error),
        "no answer: {}",
        app.status.text
    );
    assert!(app.status.text.contains("bad.drp"), "{}", app.status.text);
    assert_eq!(app.project().name, before);
}

#[test]
fn the_cut_page_arrows_change_the_track_height_and_reset_puts_it_back() {
    let mut app = app();
    let usual = app.cut_detail_row_h();
    app.step_cut_row_h(true);
    assert_eq!(app.cut_detail_row_h(), usual + 12.0);
    // It stops at the tallest and the shortest.
    for _ in 0..20 {
        app.step_cut_row_h(true);
    }
    assert_eq!(app.cut_detail_row_h(), 136.0);
    for _ in 0..20 {
        app.step_cut_row_h(false);
    }
    assert_eq!(app.cut_detail_row_h(), 40.0);
    // It is part of the layout.
    act(&mut app, Act::ResetLayout);
    assert_eq!(app.cut_detail_row_h(), usual);
}

#[test]
fn hardware_decoding_is_switched_from_the_playback_menu() {
    let mut app = app();
    assert_eq!(app.act_checked(Act::ToggleHardwareDecoding), Some(true));
    // A picture made while the card decoded.
    let project = app.project().clone();
    let content = app.render_cache.content((0, 0, 0), &project);
    let key = picasso_render::FrameKey {
        content,
        frame: 0,
        width: 16,
        height: 16,
        reduce: 1,
        alpha: false,
    };
    let frame = picasso_render::render_frame(app.project(), &app.cache, 0, 16, 16).unwrap();
    app.render_cache.put(key, std::sync::Arc::new(frame), false);
    act(&mut app, Act::ToggleHardwareDecoding);
    assert_eq!(app.act_checked(Act::ToggleHardwareDecoding), Some(false));
    assert!(!app.extra.prefs.hardware_decoding);
    // The decoders opened from now on use the processor, and the old pictures are gone.
    assert!(!picasso_decode::hw::hardware_decoding());
    assert!(app.render_cache.get(&key).is_none());
    assert!(app.status.text.contains("off"), "{}", app.status.text);
    act(&mut app, Act::ToggleHardwareDecoding);
    assert!(app.extra.prefs.hardware_decoding);
    assert_eq!(
        picasso_decode::hw::hardware_decoding(),
        std::env::var_os("PICASSO_NO_HWACCEL").is_none()
    );
}

#[test]
fn audio_scrubbing_is_switched_from_the_clip_menu() {
    let mut app = app();
    assert_eq!(app.act_checked(Act::ToggleAudioScrubbing), Some(true));
    act(&mut app, Act::ToggleAudioScrubbing);
    assert_eq!(app.act_checked(Act::ToggleAudioScrubbing), Some(false));
    assert!(!app.extra.prefs.audio_scrubbing);
    assert!(app.status.text.contains("off"), "{}", app.status.text);
    // It is a preference, so it is written down for the next run.
    assert!(
        app.extra.prefs.text().contains("audio_scrubbing=off"),
        "{}",
        app.extra.prefs.text()
    );
    act(&mut app, Act::ToggleAudioScrubbing);
    assert!(app.extra.prefs.audio_scrubbing);
    assert!(app.status.text.contains("on"), "{}", app.status.text);
}

#[test]
fn a_resolve_file_read_late_does_not_replace_a_project_opened_meanwhile() {
    let mut app = app();
    let xml = r#"<?xml version="1.0"?><xmeml><sequence><name>Cut 1</name>
        <rate><timebase>25</timebase></rate></sequence></xmeml>"#;
    app.open_bytes("cut.xml", None, xml.as_bytes().to_vec());
    // While it is read, the user opens another project.
    app.new_project();
    let opened = app.project().name.clone();
    assert!(
        wait_for(&mut app, |a| a.status.text.contains("was not opened")),
        "{}",
        app.status.text
    );
    assert_eq!(app.project().name, opened, "the project opened meanwhile stays");
}

#[test]
fn an_otio_timeline_comes_back_as_a_new_timeline() {
    let mut app = app();
    let Some(Outcome::Media(m)) = app.run(Command::AddMedia {
        name: "shot".into(),
        source: picasso_core::MediaSource::Video {
            path: "/films/shot.mov".into(),
        },
        length: Some(1000),
        default_duration: 96,
    }) else {
        panic!("media");
    };
    let v2 = app.project().tracks[1].id;
    let Some(Outcome::Clip(c)) = app.run(Command::AddClip {
        track: v2,
        media: m,
        start: 10,
        duration: Some(40),
        with_sound: false,
    }) else {
        panic!("clip");
    };
    app.run(Command::SlipClip {
        clip: c,
        source_in: 100,
    });
    let media_before = app.project().media.len();
    let timelines = app.project().timelines.len();
    let text = picasso_core::otio::to_otio(app.project());
    app.open_bytes("Cut v3.otio", None, text.into_bytes());
    assert!(!app.status.error, "{}", app.status.text);
    assert_eq!(app.project().timelines.len(), timelines + 1);
    assert_eq!(app.project().timeline_name, "Timeline 1");
    // The file is matched by its path; the demo's generated clips are made again.
    let v2 = &app.project().tracks[1];
    let back: Vec<_> = v2
        .clips
        .iter()
        .map(|c| (c.start, c.duration, c.source_in, c.media))
        .collect();
    assert_eq!(back, [(10, 40, 100, m)]);
    assert_eq!(app.project().media.len(), media_before + 3);
    // One undo takes the whole import away (the media made for it stays in the pool).
    app.undo();
    assert_eq!(app.project().timelines.len(), timelines);

    // A file that is neither in the pool nor on this computer is added offline.
    let otio = r#"{"OTIO_SCHEMA": "Timeline.1", "name": "Elsewhere", "tracks": {
        "OTIO_SCHEMA": "Stack.1", "children": [{ "OTIO_SCHEMA": "Track.1", "kind": "Video",
        "children": [{ "OTIO_SCHEMA": "Clip.1", "name": "far",
          "source_range": { "start_time": { "value": 5, "rate": 24 },
                            "duration": { "value": 20, "rate": 24 } },
          "media_reference": { "OTIO_SCHEMA": "ExternalReference.1",
            "target_url": "file:///nowhere/far%20away.mov",
            "available_range": { "start_time": { "value": 0, "rate": 24 },
                                 "duration": { "value": 50, "rate": 24 } } } }] }] } }"#;
    app.open_bytes("far.otio", None, otio.as_bytes().to_vec());
    assert!(!app.status.error, "{}", app.status.text);
    assert!(app.status.text.contains("not found"), "{}", app.status.text);
    let clip = &app.project().tracks[0].clips[0];
    let media = app.project().media(clip.media).unwrap();
    assert_eq!(media.name, "far away");
    assert_eq!(media.length, Some(50));
    assert_eq!((clip.source_in, clip.duration), (5, 20));
}

#[test]
fn bounce_selected_tracks_needs_a_selected_clip() {
    let mut app = app();
    assert!(!app.act_enabled(Act::BounceSelectedTracks));
    let first = app.project().tracks[0].clips[0].id;
    app.select_only(first);
    assert!(app.act_enabled(Act::BounceSelectedTracks));
}

#[test]
fn the_bounce_of_some_tracks_keeps_only_their_sound() {
    let mut app = app();
    let (v1, v2) = (app.project().tracks[0].id, app.project().tracks[1].id);
    let Some(Outcome::Track(a1)) = app.run(Command::AddTrack {
        kind: TrackKind::Audio,
    }) else {
        panic!()
    };
    // A solo on a track that is not bounced must not leak into the mix.
    app.run(Command::SetTrackSolo {
        track: v2,
        solo: true,
    });
    app.run(Command::SetTrackMuted {
        track: v1,
        muted: true,
    });
    let bounced = crate::menu_more::bounced(app.project(), &[a1]);
    let by_id = |id| bounced.tracks.iter().find(|t| t.id == id).unwrap();
    assert!(!by_id(a1).muted, "the chosen track stays in");
    assert!(by_id(v1).muted && by_id(v2).muted, "the others are muted");
    assert!(!by_id(v2).solo, "a solo elsewhere is cleared");
}

#[test]
fn bounce_selected_tracks_takes_the_tracks_of_the_selected_clips() {
    let mut app = app();
    let second = app.project().tracks[0].clips[1].id;
    app.select_only(second);
    let tracks = app.bounced_tracks();
    assert_eq!(tracks, vec![app.project().tracks[0].id]);
}

#[test]
fn xml_and_aaf_timelines_come_back_as_new_timelines() {
    for format in ["xml", "aaf"] {
        let mut app = app();
        let Some(Outcome::Media(m)) = app.run(Command::AddMedia {
            name: "shot".into(),
            source: picasso_core::MediaSource::Video {
                path: "/films/shot.mov".into(),
            },
            length: Some(1000),
            default_duration: 96,
        }) else {
            panic!("media");
        };
        let v2 = app.project().tracks[1].id;
        let Some(Outcome::Track(a1)) = app.run(Command::AddTrack {
            kind: TrackKind::Audio,
        }) else {
            panic!("track");
        };
        // The video on V2 and its sound on A1, at the same place.
        for track in [v2, a1] {
            let Some(Outcome::Clip(c)) = app.run(Command::AddClip {
                track,
                media: m,
                start: 10,
                duration: Some(40),
                with_sound: false,
            }) else {
                panic!("clip");
            };
            app.run(Command::SlipClip {
                clip: c,
                source_in: 100,
            });
        }
        let timelines = app.project().timelines.len();
        let bytes = match format {
            "xml" => picasso_core::xmeml::to_xmeml(app.project()).text.into_bytes(),
            _ => picasso_aaf::to_aaf(app.project()).unwrap().bytes,
        };
        app.open_bytes(&format!("Cut v3.{format}"), None, bytes);
        assert!(!app.status.error, "{format}: {}", app.status.text);
        assert_eq!(app.project().timelines.len(), timelines + 1, "{format}");
        let place = |c: &picasso_core::Clip| (c.start, c.duration, c.source_in, c.media);
        let v2 = &app.project().tracks[1];
        assert_eq!(
            v2.clips.iter().map(place).collect::<Vec<_>>(),
            [(10, 40, 100, m)],
            "{format}: the file is matched by its path"
        );
        let a1 = app
            .project()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        let sound = a1.clips.iter().find(|c| c.media == m).unwrap();
        assert_eq!(place(sound), (10, 40, 100, m), "{format}");
        // The picture and its sound come back linked, so the sound is heard once.
        assert!(v2.clips[0].link.is_some(), "{format}");
        assert_eq!(v2.clips[0].link, sound.link, "{format}");
        app.undo();
        assert_eq!(app.project().timelines.len(), timelines, "{format}: one undo");
    }
}

#[test]
fn an_xml_file_that_is_not_a_sequence_is_not_imported_as_one() {
    let mut app = app();
    let timelines = app.project().timelines.len();
    app.open_bytes(
        "empty.xml",
        None,
        b"<?xml version=\"1.0\"?><xmeml version=\"5\"><sequence><name>S</name><media><video><track/></video></media></sequence></xmeml>"
            .to_vec(),
    );
    assert_eq!(app.project().timelines.len(), timelines);
    app.open_bytes("broken.aaf", None, b"not an aaf".to_vec());
    assert!(app.status.error, "{}", app.status.text);
    assert!(app.status.text.contains("AAF"), "{}", app.status.text);
    assert_eq!(app.project().timelines.len(), timelines);
}

#[test]
fn an_export_says_an_effects_clips_grade_is_not_carried() {
    let mut app = app();
    // A clip with no file is left out of an export; without an effects clip that is all it says.
    let plain = app.left_out_note("AAF", 1);
    assert_eq!(
        plain,
        "AAF: 1 generated or compound clip(s) have no file and are left out"
    );
    let Some(Outcome::Clip(_)) = app.run(Command::CreateEffectsClip {
        start: 0,
        name: "Effects Clip 1".into(),
    }) else {
        panic!("an effects clip");
    };
    let note = app.left_out_note("XML", 1);
    assert!(
        note.contains("1 effects clip(s): their grade and effects are not carried"),
        "{note}"
    );
}

/// Runs the inbox until `done` says so (up to five seconds): a save is written on a thread.
fn process_until(app: &mut StudioApp, done: impl Fn(&StudioApp) -> bool) -> bool {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < end {
        app.process_inbox();
        if done(app) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

#[test]
fn saving_from_the_exit_box_writes_the_project_and_then_closes() {
    let mut app = app();
    let ctx = egui::Context::default();
    let path = std::env::temp_dir().join("picasso_exit_save_test.picasso");
    let _ = std::fs::remove_file(&path);
    app.project_path = Some(path.to_string_lossy().into_owned());
    assert!(app.run(Command::AddTrack { kind: TrackKind::Audio }).is_some());
    assert!(app.session.is_dirty());
    app.exit_prompt = true;
    app.save_and_exit(&ctx);
    // The box is gone and the exit waits for the file; the window does not close yet.
    assert!(!app.exit_prompt);
    assert!(app.exit_after_save);
    assert!(!app.exit_now);
    assert!(
        process_until(&mut app, |a| a.exit_now),
        "the window was not told to close after the save"
    );
    assert!(!app.exit_after_save);
    assert!(path.is_file(), "the project was not written");
    assert!(!app.session.is_dirty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_cancelled_or_failed_save_does_not_close_the_window() {
    let mut app = app();
    let ctx = egui::Context::default();
    // The place to save to was not chosen.
    app.exit_after_save = true;
    app.inbox.push(&ctx, crate::platform::Incoming::SaveCancelled);
    app.process_inbox();
    assert!(!app.exit_after_save && !app.exit_now);
    // The file could not be written.
    app.exit_after_save = true;
    app.inbox
        .push(&ctx, crate::platform::Incoming::Error("disk full".into()));
    app.process_inbox();
    assert!(!app.exit_after_save && !app.exit_now);
    // A later save of the user's own does not close the window.
    let path = std::env::temp_dir().join("picasso_exit_later_save_test.picasso");
    let _ = std::fs::remove_file(&path);
    app.project_path = Some(path.to_string_lossy().into_owned());
    app.save(&ctx, false);
    assert!(process_until(&mut app, |a| a.saved_text.is_some()));
    assert!(!app.exit_now);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn exiting_without_saving_lets_the_window_close() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.exit_prompt = true;
    app.exit_without_saving(&ctx);
    assert!(!app.exit_prompt);
}

#[test]
fn the_language_chosen_in_the_help_menu_is_kept_for_the_next_start() {
    use crate::i18n::{self, Language};
    let _only = i18n::CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut app = app();
    // English is the default, and is the one checked in the menu.
    assert_eq!(app.extra.prefs.language, Language::English);
    assert_eq!(app.act_checked(Act::Language(Language::English)), Some(true));
    assert_eq!(app.act_checked(Act::Language(Language::Spanish)), Some(false));

    act(&mut app, Act::Language(Language::Spanish));
    assert_eq!(app.extra.prefs.language, Language::Spanish);
    assert_eq!(i18n::current(), Language::Spanish);
    assert_eq!(app.act_checked(Act::Language(Language::Spanish)), Some(true));
    assert_eq!(app.act_checked(Act::Language(Language::English)), Some(false));
    assert!(app.status.text.contains("Español"), "{}", app.status.text);
    assert_eq!(i18n::menu_name("File"), "Arch.");
    assert_eq!(i18n::tr("Save Project"), "Guardar proyecto");

    // Written down, and read back at the next start.
    let text = app.extra.prefs.text();
    assert!(text.contains("language=es"), "{text}");
    let next_start = crate::prefs::Preferences::parse(&text);
    assert_eq!(next_start.language, Language::Spanish);
    next_start.apply();
    assert_eq!(i18n::current(), Language::Spanish);

    // Back to English: the labels are the English ones again.
    act(&mut app, Act::Language(Language::English));
    assert_eq!(i18n::tr("Save Project"), "Save Project");
    assert_eq!(i18n::menu_name("File"), "File");
    assert!(app.extra.prefs.text().contains("language=en"));
}
