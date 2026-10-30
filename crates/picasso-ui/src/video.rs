//! Video files: decoded with FFmpeg on the desktop, in background threads. The browser build has
//! no FFmpeg, so a video there shows the offline picture and cannot be imported.

use picasso_render::MediaCache;

/// A media cache for a new project, with the video source for this platform attached. `ctx` is
/// asked to repaint when a decoded picture arrives.
pub(crate) fn new_cache(ctx: &egui::Context) -> MediaCache {
    #[cfg(not(target_arch = "wasm32"))]
    {
        MediaCache::with_video_source(std::sync::Arc::new(native::VideoPool::new(ctx.clone())))
            .with_thumbnail_source(std::sync::Arc::new(native::VideoPool::for_thumbnails(
                ctx.clone(),
            )))
            .with_source_viewer_source(std::sync::Arc::new(native::VideoPool::new(ctx.clone())))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        MediaCache::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::fmt;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use picasso_core::MediaId;
    use picasso_decode::{DecodeError, Scrub, VideoDecoder};
    use picasso_render::{Decoded, FrameSource, VideoFrame};

    /// Pictures kept per video, so stepping back and forth, or drawing the same frame twice,
    /// does not decode again.
    const KEEP: usize = 8;
    /// How many pictures a worker decodes ahead of the last one asked for while it has nothing
    /// else to do (playing forward asks for the next picture a moment later).
    const READ_AHEAD: u8 = 3;
    /// Two requests this close, forward, count as playing.
    const PLAY_STEP_US: i64 = 200_000;
    /// An exact picture that takes longer than this to arrive is given up on.
    const WAIT_LIMIT: Duration = Duration::from_secs(20);
    /// How many bytes of key frames are kept per video. A key frame is what the viewer shows
    /// while the picture being scrubbed to is still being decoded from it; the ones seen before
    /// make going over the same stretch again instant.
    /// (About 16 key frames of 1080p decoded at half size; a multicam of nine angles stays under
    /// 300 MB.)
    const KEY_BUDGET: usize = 32 * 1024 * 1024;
    /// How many times in a row a decode may be dropped for a newer request. Past it the next one
    /// is finished whatever comes, so two clips of one video asking for different times, or a
    /// playhead that never rests, cannot keep it from ever answering.
    const MAX_CANCELS: u8 = 4;

    /// The newest thing asked of a video's worker. Only the latest request matters: while the
    /// playhead is dragged, the pictures in between are skipped.
    struct Want {
        path: String,
        time_us: i64,
        reduce: u32,
        /// How far apart the last two requests were, when that looks like playing forward.
        step_us: i64,
        /// Someone waits for exactly this picture (an export): it is never dropped.
        wait: bool,
    }

    #[derive(Default)]
    struct State {
        /// Each picture with its time and how many times smaller than the file it was made.
        frames: HashMap<MediaId, VecDeque<(i64, u32, Arc<Decoded>)>>,
        /// The key frames passed on the way to pictures that were asked for, oldest first. They
        /// only stand in for pictures near them: none is ever the exact answer.
        keys: HashMap<MediaId, VecDeque<(i64, u32, Arc<Decoded>)>>,
        want: HashMap<MediaId, Want>,
        last_asked: HashMap<MediaId, i64>,
        workers: HashSet<MediaId>,
        /// Videos that could not be opened or decoded; they show the offline picture.
        broken: HashSet<MediaId>,
        /// Whether each media's worker is currently decoding on CPU after a GPU fallback, so
        /// the status line can tell the user "decoding on processor".
        on_cpu: HashSet<MediaId>,
        /// The file each media was most recently asked for (the original or its optimised copy),
        /// so a switch between them can drop the now-stale cached frames and reopen the decoder.
        last_path: HashMap<MediaId, String>,
        closed: bool,
    }

    struct Shared {
        state: Mutex<State>,
        changed: Condvar,
        generation: AtomicU64,
        /// Pictures kept per video.
        keep: usize,
        /// Whether a video's worker ends, and closes its decoder, once it has answered its
        /// request (the media pool's thumbnails: one picture per video, once).
        one_shot: bool,
        ctx: egui::Context,
    }

    impl Shared {
        fn lock(&self) -> MutexGuard<'_, State> {
            // A panic in a worker poisons the lock; the data is still usable.
            self.state.lock().unwrap_or_else(|e| e.into_inner())
        }

        /// Wakes whoever waits for a picture and asks for a repaint.
        fn announce(&self) {
            self.generation.fetch_add(1, Ordering::Relaxed);
            self.changed.notify_all();
            self.ctx.request_repaint();
        }
    }

    /// Decodes the videos of one project, one worker thread per file, on demand.
    pub(super) struct VideoPool {
        shared: Arc<Shared>,
    }

    impl VideoPool {
        pub(super) fn new(ctx: egui::Context) -> Self {
            Self::with(ctx, KEEP, false)
        }

        /// A pool for the media pool's thumbnails: it keeps one small picture per video and does
        /// not leave a thread and an open decoder behind for each.
        pub(super) fn for_thumbnails(ctx: egui::Context) -> Self {
            Self::with(ctx, 1, true)
        }

        fn with(ctx: egui::Context, keep: usize, one_shot: bool) -> Self {
            Self {
                shared: Arc::new(Shared {
                    state: Mutex::default(),
                    changed: Condvar::new(),
                    generation: AtomicU64::new(0),
                    keep,
                    one_shot,
                    ctx,
                }),
            }
        }
    }

    impl fmt::Debug for VideoPool {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("VideoPool")
        }
    }

    impl Drop for VideoPool {
        fn drop(&mut self) {
            self.shared.lock().closed = true;
            self.shared.changed.notify_all();
        }
    }

    fn exact(state: &State, media: MediaId, time_us: i64, reduce: u32) -> Option<Arc<Decoded>> {
        state
            .frames
            .get(&media)?
            .iter()
            .find(|(t, r, _)| *t == time_us && *r == reduce)
            .map(|(_, _, f)| f.clone())
    }

    /// The picture closest in time to the one asked for, however far: while the playhead is
    /// dragged the exact picture takes a moment (the decoder starts from the key frame before it),
    /// and the viewer shows this one meanwhile instead of flashing black.
    fn nearest(state: &State, media: MediaId, time_us: i64, reduce: u32) -> Option<Arc<Decoded>> {
        let frames = state.frames.get(&media).into_iter().flatten();
        let keys = state.keys.get(&media).into_iter().flatten();
        frames
            .chain(keys)
            .filter(|(_, r, _)| *r == reduce)
            .min_by_key(|(t, _, _)| (t - time_us).abs())
            .map(|(_, _, f)| f.clone())
    }

    /// Keeps a key frame, forgetting the oldest ones past the budget. The same moment twice is
    /// kept once.
    fn keep_key(state: &mut State, media: MediaId, time_us: i64, reduce: u32, image: Arc<Decoded>) {
        let keys = state.keys.entry(media).or_default();
        keys.retain(|(t, r, _)| (*t, *r) != (time_us, reduce));
        keys.push_back((time_us, reduce, image));
        let mut bytes: usize = keys.iter().map(|(_, _, f)| f.rgba.len()).sum();
        while bytes > KEY_BUDGET && keys.len() > 1 {
            if let Some((_, _, old)) = keys.pop_front() {
                bytes -= old.rgba.len();
            }
        }
    }

    impl FrameSource for VideoPool {
        fn video_frame(
            &self,
            media: MediaId,
            path: &str,
            time_us: i64,
            reduce: u32,
            wait: bool,
        ) -> VideoFrame {
            let shared = &self.shared;
            let mut state = shared.lock();
            if state.broken.contains(&media) {
                return VideoFrame::Offline;
            }
            // The file backing this media changed (the original swapped for its optimised copy,
            // or back): the key frames and last pictures cached for the old file are not right
            // for the new one, so they are dropped and the worker is told to reopen the decoder.
            if let Some(prev) = state.last_path.get(&media) {
                if prev.as_str() != path {
                    state.frames.remove(&media);
                    state.keys.remove(&media);
                    state.last_path.insert(media, path.to_string());
                    shared.generation.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                state.last_path.insert(media, path.to_string());
            }
            if let Some(hit) = exact(&state, media, time_us, reduce) {
                return VideoFrame::Ready(hit);
            }
            let step_us = match state.last_asked.insert(media, time_us) {
                Some(before) if time_us > before && time_us - before <= PLAY_STEP_US => {
                    time_us - before
                }
                _ => 0,
            };
            state.want.insert(
                media,
                Want {
                    path: path.to_string(),
                    time_us,
                    reduce,
                    step_us,
                    wait,
                },
            );
            if state.workers.insert(media) {
                let worker = shared.clone();
                std::thread::spawn(move || run_worker(&worker, media));
            }
            shared.changed.notify_all();
            if !wait {
                return nearest(&state, media, time_us, reduce)
                    .map_or(VideoFrame::Pending, VideoFrame::StandIn);
            }
            let deadline = Instant::now() + WAIT_LIMIT;
            loop {
                if state.broken.contains(&media) {
                    return VideoFrame::Offline;
                }
                if let Some(hit) = exact(&state, media, time_us, reduce) {
                    return VideoFrame::Ready(hit);
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return VideoFrame::Offline;
                }
                // Someone else's request may have replaced ours; ask again if nothing is queued.
                state.want.entry(media).or_insert_with(|| Want {
                    path: path.to_string(),
                    time_us,
                    reduce,
                    step_us: 0,
                    wait: true,
                });
                shared.changed.notify_all();
                state = shared
                    .changed
                    .wait_timeout(state, left.min(Duration::from_millis(200)))
                    .map(|(s, _)| s)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
        }

        fn generation(&self) -> u64 {
            self.shared.generation.load(Ordering::Relaxed)
        }
    }

    /// The thread of one video: waits for a request, decodes it, then a few pictures ahead.
    fn run_worker(shared: &Shared, media: MediaId) {
        let mut decoder: Option<VideoDecoder> = None;
        let mut path = String::new();
        // The next time to read ahead, how many more after it, and the step between them.
        let mut ahead: Option<(i64, u8, i64)> = None;
        let mut reduce = 1;
        // How many decodes in a row were dropped for a newer request.
        let mut dropped: u8 = 0;
        // The last picture made: its time in the file, its size and the converted picture.
        let mut last: Option<(i64, u32, Arc<Decoded>)> = None;
        loop {
            // Whether this picture is only read ahead (nobody asked for it yet).
            let mut reading_ahead = false;
            // Whether someone waits for this picture: an export, which it is never dropped for.
            let waited_for;
            let time_us = {
                let mut state = shared.lock();
                loop {
                    if state.closed {
                        return;
                    }
                    if let Some(w) = state.want.remove(&media) {
                        path = w.path;
                        reduce = w.reduce;
                        waited_for = w.wait;
                        ahead = (w.step_us > 0).then_some((
                            w.time_us + w.step_us,
                            READ_AHEAD,
                            w.step_us,
                        ));
                        break w.time_us;
                    }
                    if let Some((next, left, step)) = ahead {
                        ahead = (left > 1).then_some((next + step, left - 1, step));
                        if exact(&state, media, next, reduce).is_none() {
                            reading_ahead = true;
                            waited_for = false;
                            break next;
                        }
                        continue;
                    }
                    if shared.one_shot {
                        // Nothing more is wanted of this video: a later request starts a new
                        // worker (it is taken out of the set while the lock is held).
                        state.workers.remove(&media);
                        return;
                    }
                    state = shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(|e| e.into_inner());
                }
            };
            if exact(&shared.lock(), media, time_us, reduce).is_some() {
                continue;
            }
            // Hardware decoding was turned on or off: open the file again the new way.
            if decoder.as_ref().is_some_and(VideoDecoder::is_stale) {
                decoder = None;
            }
            // The file this media reads from changed (a copy was made, or "Use Optimised Media"
            // was toggled) since the decoder was opened: reopen it so the right file is decoded.
            if decoder
                .as_ref()
                .is_some_and(|d| d.path() != Path::new(&path))
            {
                decoder = None;
            }
            if decoder.is_none() {
                match VideoDecoder::open(Path::new(&path)) {
                    Ok(d) => decoder = Some(d),
                    Err(e) => {
                        tracing::warn!("cannot open {path}: {e}");
                        fail(shared, media);
                        return;
                    }
                }
            }
            let Some(d) = decoder.as_mut() else { return };
            // Track whether this decoder fell back to the CPU, so the status line can warn.
            if !d.on_card() {
                shared.lock().on_cpu.insert(media);
                crate::session_log::note(
                    "WARN",
                    &format!(
                        "{} fell back to processor decoding (GPU decode failed)",
                        path
                    ),
                );
            }
            // The same picture as the last one (a project that plays faster than its video asks
            // for each picture several times): keep that answer, convert nothing.
            if let (Some(held), Some((at, size, image))) = (d.held_picture_at(time_us), &last) {
                if *at == held && *size == reduce {
                    let mut state = shared.lock();
                    let queue = state.frames.entry(media).or_default();
                    queue.push_back((time_us, reduce, image.clone()));
                    while queue.len() > shared.keep {
                        queue.pop_front();
                    }
                    drop(state);
                    shared.announce();
                    continue;
                }
            }
            // A newer request replaces this one: stop at once instead of decoding on to a
            // picture nobody wants any more. On the way, the key frame it starts from is shown
            // while the rest is decoded.
            let may_drop = !waited_for && dropped < MAX_CANCELS;
            let cancel = || {
                let state = shared.lock();
                state.closed
                    || (may_drop
                        && state
                            .want
                            .get(&media)
                            .is_some_and(|w| w.time_us != time_us || w.reduce != reduce))
            };
            let mut preview = |frame: picasso_decode::Frame| {
                if shared.one_shot {
                    return;
                }
                let image = Arc::new(Decoded {
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba,
                });
                keep_key(&mut shared.lock(), media, frame.pts_us, reduce, image);
                shared.announce();
            };
            let mut decoded = d.frame_at_scrub(
                time_us,
                reduce,
                &mut Scrub {
                    cancel: &cancel,
                    preview: &mut preview,
                },
            );
            if matches!(decoded, Err(DecodeError::Cancelled)) {
                // The newer request is waiting at the top of the loop.
                dropped += 1;
                continue;
            }
            dropped = 0;
            if decoded.is_err() && reading_ahead {
                // Past the end of the file (playing up to the last frame): stop reading ahead.
                ahead = None;
                continue;
            }
            if decoded.is_err() {
                // A decoder left at the end of the file can fail to seek back (a loop to the
                // start): open the file again and try once more.
                decoder = VideoDecoder::open(Path::new(&path)).ok();
                if let Some(d) = decoder.as_mut() {
                    decoded = d.frame_at_reduced(time_us, reduce);
                }
            }
            if decoded.is_err() {
                // A time past the last picture keeps showing the nearest one.
                let mut state = shared.lock();
                let near = state
                    .frames
                    .get(&media)
                    .and_then(|q| {
                        q.iter()
                            .filter(|(_, r, _)| *r == reduce)
                            .max_by_key(|(t, _, _)| *t)
                    })
                    .filter(|(t, _, _)| *t < time_us)
                    .map(|(_, _, f)| f.clone());
                if let Some(image) = near {
                    let queue = state.frames.entry(media).or_default();
                    queue.push_back((time_us, reduce, image));
                    while queue.len() > shared.keep {
                        queue.pop_front();
                    }
                    drop(state);
                    shared.announce();
                    continue;
                }
            }
            match decoded {
                Ok(frame) => {
                    let pts_us = frame.pts_us;
                    let image = Arc::new(Decoded {
                        width: frame.width,
                        height: frame.height,
                        rgba: frame.rgba,
                    });
                    last = Some((pts_us, reduce, image.clone()));
                    let mut state = shared.lock();
                    let queue = state.frames.entry(media).or_default();
                    queue.push_back((time_us, reduce, image));
                    while queue.len() > shared.keep {
                        queue.pop_front();
                    }
                    drop(state);
                    shared.announce();
                }
                Err(e) => {
                    tracing::warn!("cannot decode at {time_us} us: {e}");
                    fail(shared, media);
                    return;
                }
            }
        }
    }

    fn fail(shared: &Shared, media: MediaId) {
        shared.lock().broken.insert(media);
        shared.announce();
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Eight seconds of 640x360 with a key frame only every 250 pictures; `None` when the
        /// `ffmpeg` program is not installed.
        fn long_gop(name: &str) -> Option<String> {
            let path = std::env::temp_dir().join(name);
            std::process::Command::new("ffmpeg")
                .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
                .arg("testsrc=duration=8:size=640x360:rate=25")
                .args(["-pix_fmt", "yuv420p", "-g", "250", "-keyint_min", "250"])
                .args(["-sc_threshold", "0"])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success())
                .then(|| path.to_string_lossy().into_owned())
        }

        /// Two seconds of `size` at 25 fps; `None` when `ffmpeg` is not installed.
        fn video_size(name: &str, size: &str) -> Option<String> {
            let path = std::env::temp_dir().join(name);
            std::process::Command::new("ffmpeg")
                .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
                .arg(format!("testsrc2=duration=2:size={size}:rate=25"))
                .args(["-pix_fmt", "yuv420p", "-g", "25"])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success())
                .then_some(path.to_string_lossy().into_owned())
        }

        /// Asks for the picture until it is the exact one (up to ten seconds).
        fn until_ready(pool: &VideoPool, media: MediaId, path: &str, t: i64) -> bool {
            let end = Instant::now() + Duration::from_secs(10);
            while Instant::now() < end {
                if matches!(
                    pool.video_frame(media, path, t, 1, false),
                    VideoFrame::Ready(_)
                ) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            false
        }

        /// Plays `path` (60 fps) as the viewer asks for it, in real time from `from_us` for
        /// `seconds`, and notes what each request drew: the time of the picture it got (the exact
        /// one, or the stand-in's own time), or `None` when nothing was there (black).
        fn play(
            pool: &VideoPool,
            media: MediaId,
            path: &str,
            from_us: i64,
            seconds: f64,
            speed: f64,
        ) -> Vec<(i64, Option<i64>)> {
            let start = Instant::now();
            let mut drawn = Vec::new();
            loop {
                let t = from_us
                    + (start.elapsed().as_secs_f64() * 60.0 * speed) as i64 * 1_000_000 / 60;
                if t > from_us + (seconds * 1e6) as i64 {
                    break;
                }
                let got = match pool.video_frame(media, path, t, 2, false) {
                    VideoFrame::Ready(_) => Some(t),
                    VideoFrame::StandIn(image) => {
                        let state = pool.shared.lock();
                        let all = state.frames.get(&media).into_iter().flatten();
                        let keys = state.keys.get(&media).into_iter().flatten();
                        all.chain(keys)
                            .find(|(_, _, f)| Arc::ptr_eq(f, &image))
                            .map(|(at, _, _)| *at)
                    }
                    _ => None,
                };
                drawn.push((t, got));
                std::thread::sleep(Duration::from_millis(16));
            }
            drawn
        }

        /// How often the picture drawn went back in time, how many requests drew nothing, and
        /// how many drew a stand-in later than the time asked for.
        fn judge(drawn: &[(i64, Option<i64>)]) -> (usize, usize, usize) {
            let mut last = i64::MIN;
            let (mut back, mut black, mut future) = (0, 0, 0);
            for (t, got) in drawn {
                match got {
                    Some(at) => {
                        if *at < last {
                            back += 1;
                        }
                        if at > t {
                            future += 1;
                        }
                        last = last.max(*at);
                    }
                    None => black += 1,
                }
            }
            (back, black, future)
        }

        /// Plays 4K in real time for about 12 seconds: run it with `--ignored` when playback is in
        /// doubt.
        #[test]
        #[ignore]
        fn playing_draws_pictures_in_order_and_never_black_once_started() {
            let path = std::env::temp_dir().join("picasso_pool_play_4k.mp4");
            let made = std::process::Command::new("ffmpeg")
                .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
                .arg("testsrc2=duration=8:size=2160x3840:rate=60")
                .args([
                    "-pix_fmt", "yuv420p", "-g", "120", "-c:v", "libx264", "-preset", "veryfast",
                ])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success());
            if !made {
                return;
            }
            let path = path.to_string_lossy().into_owned();
            let pool = VideoPool::new(egui::Context::default());
            let media = MediaId(3);
            // The first picture, as when the playhead rests before Play is pressed.
            assert!(until_ready_at(&pool, media, &path, 0, 2));
            let drawn = play(&pool, media, &path, 0, 4.0, 1.0);
            let (back, black, future) = judge(&drawn);
            eprintln!(
                "PLAY back={back} black={black} future={future} ticks={}",
                drawn.len()
            );
            // Faster than the pictures can be decoded, as on a slower computer.
            let pool = VideoPool::new(egui::Context::default());
            assert!(until_ready_at(&pool, media, &path, 0, 2));
            let slow = play(&pool, media, &path, 0, 7.5, 4.0);
            let (b, k, f) = judge(&slow);
            let exact = slow.iter().filter(|(t, got)| *got == Some(*t)).count();
            eprintln!(
                "PLAY4x back={b} black={k} future={f} exact={exact} ticks={}",
                slow.len()
            );
            let _ = std::fs::remove_file(&path);
            assert_eq!(back, 0, "the picture went back in time: {drawn:?}");
            assert_eq!(black, 0, "nothing was drawn: {drawn:?}");
        }

        fn until_ready_at(
            pool: &VideoPool,
            media: MediaId,
            path: &str,
            t: i64,
            reduce: u32,
        ) -> bool {
            let end = Instant::now() + Duration::from_secs(10);
            while Instant::now() < end {
                if matches!(
                    pool.video_frame(media, path, t, reduce, false),
                    VideoFrame::Ready(_)
                ) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            false
        }

        #[test]
        fn the_key_frame_stands_in_while_a_far_picture_is_decoded() {
            let Some(path) = long_gop("picasso_pool_long_gop.mp4") else {
                return;
            };
            let pool = VideoPool::new(egui::Context::default());
            let media = MediaId(1);
            assert!(until_ready(&pool, media, &path, 6_000_000));
            // The picture at the start of the file was passed on the way and is kept to stand in
            // for anything near it, though it was never asked for.
            let kept = pool.shared.lock().keys.get(&media).map(|k| k.len());
            assert_eq!(kept, Some(1));
            let near = nearest(&pool.shared.lock(), media, 100_000, 1);
            assert!(
                near.is_some(),
                "a picture near the start should have a stand-in"
            );
            let _ = std::fs::remove_file(path);
        }

        #[test]
        fn the_last_request_is_answered_after_a_burst_of_others() {
            let Some(path) = long_gop("picasso_pool_burst.mp4") else {
                return;
            };
            let pool = VideoPool::new(egui::Context::default());
            let media = MediaId(2);
            // The playhead is thrown about faster than the pictures can be decoded.
            for i in 0..40 {
                let t = if i % 2 == 0 { 7_000_000 } else { 3_000_000 } + i * 40_000;
                pool.video_frame(media, &path, t, 1, false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                until_ready(&pool, media, &path, 5_000_000),
                "it never settled"
            );
            // And an export that waits for each picture is not dropped for the others.
            let blocking = pool.video_frame(media, &path, 2_000_000, 1, true);
            assert!(matches!(blocking, VideoFrame::Ready(_)));
            let _ = std::fs::remove_file(path);
        }

        /// Swapping the file a media reads from (the original for its optimised copy, or back)
        /// must reopen the decoder and drop the old file's cached frames, not keep serving them.
        #[test]
        fn switching_the_backing_file_rereads_the_new_one() {
            let big = video_size("picasso_pool_switch_big.mp4", "1280x720");
            let small = video_size("picasso_pool_switch_small.mp4", "320x180");
            let (Some(big), Some(small)) = (big, small) else {
                return; // no ffmpeg here
            };
            let pool = VideoPool::new(egui::Context::default());
            let media = MediaId(9);
            // Prime the cache with the big file; the worker opens a decoder on it.
            assert!(
                until_ready_at(&pool, media, &big, 0, 1),
                "the big file decoded"
            );
            // Now the small file backs the same media: an optimised copy, in effect.
            assert!(
                until_ready_at(&pool, media, &small, 0, 1),
                "the small file decoded after the switch"
            );
            // The exact picture asked for must be the small file's, not the cached big one.
            match pool.video_frame(media, &small, 0, 1, true) {
                VideoFrame::Ready(f) => assert_eq!(
                    (f.width, f.height),
                    (320, 180),
                    "the switch read the new file, not the cached old one"
                ),
                other => panic!("expected a ready frame, got {other:?}"),
            }
            let _ = std::fs::remove_file(&big);
            let _ = std::fs::remove_file(&small);
        }
    }
}
