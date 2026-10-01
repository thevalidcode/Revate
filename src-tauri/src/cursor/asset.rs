//! The cursor image, and the handful of ways it can be styled.
//!
//! # Why the arrow is drawn, not loaded
//!
//! The obvious move is to grab a PNG and composite it. That has two problems on
//! macOS: the system arrow ships at 1×/2× against a cursor *size* the user
//! configured, and its hotspot is at the tip in one and centre-ish in the other,
//! so a loaded bitmap lands visibly off the click point. `assets/cursor.png` is
//! supported for anyone who wants a custom shape, but the built-in shape is a
//! vector arrow with an explicit hotspot — it stays crisp at any zoom and its
//! anchor is unambiguous.

use serde::{Deserialize, Serialize};

/// The arrow's size in video pixels at 1×.
pub const CURSOR_WIDTH: f64 = 28.0;
/// The arrow's height in video pixels at 1×.
pub const CURSOR_HEIGHT: f64 = 40.0;

/// Where the tip sits inside the arrow's bounding box, as a fraction.
///
/// (0, 0) — the top-left corner — is the hotspot, so a click is always drawn at
/// the exact recorded point.
pub const HOTSPOT: (f64, f64) = (0.0, 0.0);

/// How a cursor is drawn at a given moment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorStyle {
    /// Multiplier on the 1× size.
    pub scale: f64,
    /// RGBA, 0-255. This is the *fill* of the arrow body.
    pub fill: [u8; 4],
    /// RGBA outline, drawn one pixel outside the body. This is what keeps the
    /// cursor readable over both white documents and dark code editors.
    pub outline: [u8; 4],
    /// RGBA of the click ripple ring.
    pub ripple: [u8; 4],
    /// Ripple radius at full expansion, in video pixels.
    pub ripple_radius: f64,
}

impl Default for CursorStyle {
    /// A white arrow with a dark outline, and a soft accent ripple.
    ///
    /// White-with-black-outline is the one combination that stays legible over
    /// arbitrary screen content without tinting, which matters because a
    /// screencast has no idea what is underneath.
    fn default() -> Self {
        Self {
            scale: 1.0,
            fill: [255, 255, 255, 255],
            outline: [17, 17, 17, 255],
            ripple: [59, 130, 246, 255],
            ripple_radius: 34.0,
        }
    }
}

impl CursorStyle {
    /// The style used inside a zoom segment.
    ///
    /// Zoomed in, the content is larger but the cursor would be too, if it were
    /// scaled with the crop — so instead it is scaled *against* it, keeping a
    /// near-constant on-screen size, and given a heavier outline for contrast
    /// against whatever the zoom brought closer.
    pub fn for_zoom(zoom_level: f64) -> Self {
        let base = Self::default();
        Self {
            // Partial compensation: 1× at 1×, ~1.16× at 1.8×. Full
            // compensation (dividing by zoom) would make the cursor tiny in a
            // heavily zoomed passage, which reads as a bug.
            scale: base.scale * (1.0 + (zoom_level - 1.0) * 0.1),
            fill: base.fill,
            outline: [10, 10, 10, 255],
            ripple: base.ripple,
            ripple_radius: base.ripple_radius * 1.2,
        }
    }

    /// Scale clamped to something sane — a pathological segment must not produce
    /// a cursor the size of the frame.
    pub fn clamped_scale(&self) -> f64 {
        self.scale.clamp(0.25, 6.0)
    }
}

/// An RGBA pixel.
pub type Rgba = [u8; 4];

/// Source-over alpha compositing of a single pixel.
///
/// Straight (non-premultiplied) alpha, so `out = src*src_a + dst*(1-src_a)`.
/// Integer math with rounding, because a 0.5 alpha over a dark background that
/// composites to 0 or 1 shows up as a crawling fringe on a moving cursor.
pub fn blend(dst: &mut Rgba, src: Rgba) {
    let src_a = src[3] as u32;
    if src_a == 0 {
        return;
    }
    if src_a == 255 {
        *dst = src;
        return;
    }
    let dst_a = dst[3] as u32;

    // Composite onto transparent black first when the backdrop is clear, so a
    // cursor drawn on an empty overlay layer keeps its own colour.
    let inv = 255 - src_a;
    let out_a = src_a + dst_a * inv / 255;
    let mut out = [0u8; 4];
    for channel in 0..3 {
        // Weight each side by its own alpha, exactly as `out = src*src_a +
        // dst*(1-src_a)` says, then divide back out by the *resulting* alpha to
        // return to straight alpha. Dividing by a constant 255 instead would
        // leave the colour premultiplied, so a 50%-alpha pixel over an empty
        // layer would come out at half its brightness rather than its own.
        let s = src[channel] as u32 * src_a;
        let d = dst[channel] as u32 * dst_a;
        let o = s + d * inv / 255;
        out[channel] = ((o + out_a / 2) / out_a).min(255) as u8;
    }
    out[3] = out_a.min(255) as u8;
    *dst = out;
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opaque_pixel_replaces_the_backdrop() {
        let mut dst: Rgba = [0, 0, 0, 255];
        blend(&mut dst, [255, 0, 0, 255]);
        assert_eq!(dst, [255, 0, 0, 255]);
    }

    #[test]
    fn a_transparent_pixel_changes_nothing() {
        let mut dst: Rgba = [10, 20, 30, 255];
        blend(&mut dst, [255, 255, 255, 0]);
        assert_eq!(dst, [10, 20, 30, 255]);
    }

    #[test]
    fn a_half_opaque_pixel_mixes() {
        let mut dst: Rgba = [0, 0, 0, 255];
        blend(&mut dst, [255, 255, 255, 128]);
        assert_eq!(dst, [128, 128, 128, 255]);
    }

    #[test]
    fn blending_onto_a_clear_layer_keeps_the_source_colour() {
        let mut dst: Rgba = [0, 0, 0, 0];
        blend(&mut dst, [255, 0, 0, 128]);
        assert_eq!(&dst[..3], &[255, 0, 0], "colour should survive");
        assert_eq!(dst[3], 128, "alpha should be the source alpha");
    }

    #[test]
    fn the_default_style_is_a_legible_arrow() {
        let style = CursorStyle::default();
        assert_eq!(style.fill, [255, 255, 255, 255]);
        assert_eq!(style.outline[3], 255, "the outline must be solid");
    }

    #[test]
    fn zoom_grows_the_cursor_a_little_and_never_enormously() {
        let base = CursorStyle::default();
        let zoomed = CursorStyle::for_zoom(2.0);
        assert!(zoomed.scale > base.scale, "should compensate somewhat");
        assert!(zoomed.scale < 2.0, "but not proportionally");

        let extreme = CursorStyle {
            scale: 100.0,
            ..CursorStyle::default()
        };
        assert_eq!(extreme.clamped_scale(), 6.0);
    }

    #[test]
    fn the_hotspot_is_the_tip_so_clicks_land_on_the_point() {
        assert_eq!(HOTSPOT, (0.0, 0.0));
    }
}
