//! Motion tracking maths.
//!
//! * [`PointTracker`] follows one point from frame to frame (normalised cross-correlation over an
//!   image pyramid, then a Gauss-Newton refinement for sub-pixel accuracy).
//! * [`PlanarTracker`] follows a four-corner region (a flat surface) with a grid of such patches
//!   and a robust [`Homography`] fit.
//! * [`stabilize`] measures the camera movement between frames.
//!
//! Textbook methods: template matching, Lucas-Kanade style translation estimation and the
//! normalised direct linear transform of Hartley and Zisserman. Nothing here does I/O, starts a
//! thread or can panic, so it runs on any thread; callers hand it grey pictures ([`GrayImage`]).

pub mod homography;
pub mod image;
pub mod matcher;
pub mod planar;
pub mod point;
pub mod stabilize;

pub use homography::{
    estimate_homography, homography_from_points, homography_from_quads, is_convex_quad, quad_area,
    quad_centre, similarity_from_points, similarity_from_quads, Homography, HomographyFit, Quad,
    RansacSettings, Similarity,
};
pub use image::{GrayImage, Vec2};
pub use planar::{PlanarResult, PlanarSettings, PlanarTracker};
pub use point::{PointTracker, TrackResult, TrackerSettings};

#[cfg(test)]
pub(crate) mod testutil;
