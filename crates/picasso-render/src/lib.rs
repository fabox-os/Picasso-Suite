//! Renders a frame of a project.
//!
//! The same code makes the viewer's picture and the exported frames, so an export is the
//! viewer's picture at full size. Tracks are composited from the bottom up; each clip's media is
//! drawn, graded and blended with its opacity. Everything is on the CPU and has no platform
//! code, so it runs the same natively and in WebAssembly.

mod cache;
mod compose;
mod frames;
mod source;
mod text;
mod transform;
pub mod transitions;

pub use cache::{Decoded, FrameSource, MediaCache, VideoFrame};
pub use compose::{
    encode_png, export_sequence, render_frame, render_frame_alpha, render_frame_exact,
    render_media_frame, render_thumbnail, render_thumbnail_exact, Frame, RenderError,
};
pub use frames::{content_hash, FrameCache, FrameKey};
