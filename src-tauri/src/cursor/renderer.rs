//! Drawing the cursor into RGBA pixels.
//!
//! [`Frame`] is a plain `width × height` RGBA buffer with a handful of drawing
//! primitives. It is not a general 2D library and does not try to be: it needs to
//! put one shape on one transparent layer, thousands of times, fast enough to
//! keep up with a live preview and an export alike.
//!
//! [`render_at`] is the layer above that: it answers "what does the cursor layer
//! look like at time *t*", which is the one question both the preview and the
//! exporter ask.
//!
//! # The arrow
//!
//! Drawn as a filled polygon (the classic macOS pointer outline) with a
//! one-pixel outline stroked around it. The outline is not decoration — a white
//! arrow vanishes on a white document, and a screencast cannot know what is
//! behind it.
//!
//! # The ripple
//!
//! An expanding ring on click, fading as it grows. The edge is antialiased by
//! supersampling the radius test, which is what stops a fast-expanding circle
//! from strobing.

use super::asset::{blend, CursorStyle, Rgba, CURSOR_HEIGHT, CURSOR_WIDTH, HOTSPOT};

/// A transparent RGBA image.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, no padding.
    pub data: Vec<u8>,
}

impl Frame {
    /// A fully transparent frame.
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize) * (height as usize) * 4;
        Self {
            width,
            height,
            data: vec![0u8; len],
        }
    }

    /// Clear back to fully transparent, reusing the allocation.
    pub fn clear(&mut self) {
        self.data.iter_mut().for_each(|b| *b = 0);
    }

    #[inline]
    fn in_bounds(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && x < self.width as i64 && y < self.height as i64
    }

    /// Composite one pixel, ignoring out-of-bounds coordinates.
    pub fn put(&mut self, x: i64, y: i64, color: Rgba) {
        if !self.in_bounds(x, y) {
            return;
        }
        let index = (y as usize * self.width as usize + x as usize) * 4;
        let mut pixel = [
            self.data[index],
            self.data[index + 1],
            self.data[index + 2],
            self.data[index + 3],
        ];
        blend(&mut pixel, color);
        self.data[index..index + 4].copy_from_slice(&pixel);
    }

    /// Read one pixel, or transparent black if it is off-frame.
    pub fn get(&self, x: i64, y: i64) -> Rgba {
        if !self.in_bounds(x, y) {
            return [0, 0, 0, 0];
        }
        let index = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.data[index],
            self.data[index + 1],
            self.data[index + 2],
            self.data[index + 3],
        ]
    }

    /// Any pixel with a non-zero alpha.
    pub fn is_empty(&self) -> bool {
        self.data.chunks_exact(4).all(|p| p[3] == 0)
    }

    /// Draw the cursor arrow with its hotspot at `(x, y)`.
    ///
    /// Coordinates may fall outside the frame; the arrow is clipped rather than
    /// skipped, so a cursor at the edge of the video is still half-visible
    /// instead of popping out of existence.
    pub fn draw_cursor(&mut self, x: f64, y: f64, style: &CursorStyle) {
        let scale = style.clamped_scale();
        let w = CURSOR_WIDTH * scale;
        let h = CURSOR_HEIGHT * scale;

        // The hotspot is the tip, so the arrow's box hangs off the point.
        let left = (x - HOTSPOT.0 * w).floor() as i64;
        let top = (y - HOTSPOT.1 * h).floor() as i64;

        // Walk the bounding box once, painting the outline ring and the fill in
        // the same pass: a pixel inside the shape gets the fill, a pixel just
        // outside it gets the outline. Two passes would test every pixel twice,
        // and this runs once per drawn frame.
        //
        // Coverage is decided at the pixel's *centre*, not its corner: a polygon
        // edge is a continuous curve, and testing the corner would put the
        // hotspot itself outside its own arrow — a click would read as a dark
        // notch instead of the tip of the pointer.
        for dy in -1..=(h as i64) {
            for dx in -1..=(w as i64) {
                let px = dx as f64 + 0.5;
                let py = dy as f64 + 0.5;
                if in_polygon(px, py, w, h, scale) {
                    self.put(left + dx, top + dy, style.fill);
                } else if within_distance(px, py, w, h, scale) <= 1.0 {
                    self.put(left + dx, top + dy, style.outline);
                }
            }
        }
    }

    /// Draw a click ripple centred at `(x, y)`.
    ///
    /// `progress` runs 0 → 1 across the ripple's life: the ring grows from
    /// nothing to `style.ripple_radius` while fading out. The ring edge is
    /// antialiased by sampling four points per pixel against the radius, which is
    /// what keeps a fast-expanding circle from strobing on the way out.
    pub fn draw_ripple(&mut self, x: f64, y: f64, progress: f64, style: &CursorStyle) {
        let progress = progress.clamp(0.0, 1.0);
        if progress >= 1.0 {
            return;
        }

        // Ease out, so it leaves slowly and does not snap away at the end.
        let eased = 1.0 - (1.0 - progress).powi(2);
        let radius = style.ripple_radius * eased;
        // Opacity falls off faster than the radius grows.
        let alpha = ((1.0 - progress) * 255.0 * 0.75) as u8;
        if alpha == 0 || radius <= 0.0 {
            return;
        }

        let color = [style.ripple[0], style.ripple[1], style.ripple[2], alpha];
        // Ring thickness thins as it expands, the way a real ripple does.
        let thickness = (2.5 - 1.0 * eased).max(1.0);

        let min_x = (x - radius - 2.0).floor() as i64;
        let max_x = (x + radius + 2.0).ceil() as i64;
        let min_y = (y - radius - 2.0).floor() as i64;
        let max_y = (y + radius + 2.0).ceil() as i64;

        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let dx = px as f64 + 0.5 - x;
                let dy = py as f64 + 0.5 - y;

                // 4× supersample around the pixel centre for a soft edge.
                let mut hits = 0;
                for (ox, oy) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                    let sx = dx + ox;
                    let sy = dy + oy;
                    if ((sx * sx + sy * sy).sqrt() - radius).abs() <= thickness {
                        hits += 1;
                    }
                }
                if hits == 0 {
                    continue;
                }
                let mut pixel = color;
                pixel[3] = (alpha as u32 * hits as u32 / 4) as u8;
                self.put(px, py, pixel);
            }
        }
    }
}

/// Is `(x, y)` inside the arrow shape, in the arrow's own coordinates?
///
/// The pointer is a polygon — tip at the top-left, a wide base, and the notch at
/// the bottom-left. Using an explicit point list rather than a bitmap keeps it
/// sharp at any scale, which matters because the style scale changes with zoom.
fn in_polygon(x: f64, y: f64, w: f64, h: f64, scale: f64) -> bool {
    point_in_polygon(x, y, &arrow_points(w, h)) || in_tail(x, y, w, h, scale)
}

/// The outline of a macOS-style pointer, in unit coordinates of its own box.
fn arrow_points(w: f64, h: f64) -> Vec<(f64, f64)> {
    vec![
        (0.0, 0.0),          // tip — the hotspot
        (0.0, h * 0.78),      // down the left edge
        (w * 0.26, h * 0.52), // the notch
        (w * 0.44, h),        // right foot
        (w * 0.66, h * 0.87), // bottom-right bevel
        (w * 0.48, h * 0.40), // back up to the waist
        (w, h * 0.40),        // the wing tip
    ]
}

/// The rounded tail below the waist, only once the arrow is big enough to show one.
fn in_tail(x: f64, y: f64, w: f64, h: f64, scale: f64) -> bool {
    if scale < 1.2 {
        return false;
    }
    let dx = x - w * 0.30;
    let dy = y - h * 0.66;
    let r = w * 0.10;
    dx * dx + dy * dy <= r * r
}

/// Distance from `(x, y)` to the nearest edge of the arrow, for the outline pass.
fn within_distance(x: f64, y: f64, w: f64, h: f64, scale: f64) -> f64 {
    if in_tail(x, y, w, h, scale) {
        return 0.0;
    }
    let points = arrow_points(w, h);
    let mut best = f64::MAX;
    for i in 0..points.len() {
        let (x1, y1) = points[i];
        let (x2, y2) = points[(i + 1) % points.len()];
        best = best.min(distance_to_segment(x, y, x1, y1, x2, y2));
    }
    best
}

/// Even-odd point-in-polygon, counting edge crossings along a ray toward +x.
fn point_in_polygon(x: f64, y: f64, points: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = points.len();
    for i in 0..n {
        let (x1, y1) = points[i];
        let (x2, y2) = points[(i + 1) % n];
        if (y1 > y) != (y2 > y) {
            let t = (y - y1) / (y2 - y1);
            if x < x1 + t * (x2 - x1) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Render the cursor layer at time `t`.
///
/// This is the single entry point shared by the preview and the exporter, which
/// is what guarantees they agree: same track, same segments, same style
/// resolution, same rasterizer.
///
/// The cursor is positioned in the layer's own (output) coordinates, so a caller
/// compositing over a zoomed crop must hand in a layer sized to the cropped
/// region and `viewport` describing it.
pub fn render_at(
    frame: &mut Frame,
    track: &crate::input::cursor::CursorTrack,
    segments: &[crate::zoom::planner::ZoomSegment],
    viewport: &crate::zoom::viewport::Viewport,
    t: f64,
) {
    frame.clear();

    let Some((x, y)) = track.position_at(t) else {
        return;
    };

    // The layer may be a cropped, scaled view of the source, so the recorded
    // position has to travel through the same transform the video did.
    let scale_x = frame.width as f64 / viewport.width;
    let scale_y = frame.height as f64 / viewport.height;
    let (px, py) = ((x - viewport.x) * scale_x, (y - viewport.y) * scale_y);

    // Off the visible area entirely — the pointer was on another monitor, or
    // the zoom moved the frame off it. Nothing to draw.
    let margin = CURSOR_WIDTH * 4.0;
    if px < -margin || py < -margin || px > frame.width as f64 + margin || py > frame.height as f64 + margin {
        return;
    }

    // Per-segment style: inside a zoom the cursor gets the zoom's treatment.
    let style = match crate::zoom::planner::segment_at(segments, t) {
        Some(segment) => CursorStyle::for_zoom(segment.zoom_level),
        None => CursorStyle::default(),
    };

    if let Some(ripple) = track.ripple_at(t) {
        let (rx, ry) = (
            (ripple.x - viewport.x) * scale_x,
            (ripple.y - viewport.y) * scale_y,
        );
        frame.draw_ripple(rx, ry, ripple.progress, &style);
    }

    frame.draw_cursor(px, py, &style);
}

/// Distance from `(x, y)` to the segment `(x1, y1)–(x2, y2)`.
///
/// The projection onto the segment's direction is clamped to `[0, 1]`, so a point
/// beyond either end is measured from that endpoint rather than from the
/// infinite line. A zero-length segment has no direction and degrades to a point
/// distance instead of dividing by zero.
fn distance_to_segment(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let length_sq = dx * dx + dy * dy;
    if length_sq < f64::EPSILON {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }
    let t = (((px - x1) * dx + (py - y1) * dy) / length_sq).clamp(0.0, 1.0);
    let cx = x1 + t * dx;
    let cy = y1 + t * dy;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_frame_is_fully_transparent() {
        let frame = Frame::new(8, 4);
        assert_eq!(frame.data.len(), 8 * 4 * 4);
        assert!(frame.is_empty());
    }

    #[test]
    fn drawing_the_cursor_makes_it_not_empty() {
        let mut frame = Frame::new(200, 200);
        frame.draw_cursor(100.0, 100.0, &CursorStyle::default());
        assert!(!frame.is_empty(), "nothing was drawn");
        // The hotspot is the tip: (100,100) itself is opaque fill.
        assert_eq!(frame.get(100, 100), [255, 255, 255, 255]);
    }

    #[test]
    fn the_outline_is_drawn_just_outside_the_fill() {
        let mut frame = Frame::new(200, 200);
        frame.draw_cursor(100.0, 100.0, &CursorStyle::default());

        // Walk down the left edge of the arrow: the fill sits at x=100, and the
        // dark outline is immediately to its left.
        let outline = frame.get(99, 102);
        assert_eq!(outline, [17, 17, 17, 255], "expected the dark outline");
    }

    #[test]
    fn a_cursor_at_the_frame_edge_is_clipped_not_dropped() {
        let mut frame = Frame::new(60, 60);
        frame.draw_cursor(2.0, 2.0, &CursorStyle::default());
        assert!(!frame.is_empty(), "should still draw its corner");

        let mut far = Frame::new(60, 60);
        far.draw_cursor(-500.0, -500.0, &CursorStyle::default());
        assert!(far.is_empty(), "entirely off-frame draws nothing");
    }

    #[test]
    fn drawing_survives_degenerate_frames() {
        let mut frame = Frame::new(0, 0);
        frame.draw_cursor(10.0, 10.0, &CursorStyle::default());
        frame.draw_ripple(10.0, 10.0, 0.5, &CursorStyle::default());
        assert!(frame.is_empty());
    }

    #[test]
    fn clear_resets_to_transparent() {
        let mut frame = Frame::new(50, 50);
        frame.draw_cursor(25.0, 25.0, &CursorStyle::default());
        assert!(!frame.is_empty());
        frame.clear();
        assert!(frame.is_empty());
    }

    #[test]
    fn a_ripple_grows_and_fades() {
        let style = CursorStyle::default();

        let mut early = Frame::new(120, 120);
        early.draw_ripple(60.0, 60.0, 0.1, &style);
        let early_pixels = early.data.chunks_exact(4).filter(|p| p[3] > 0).count();

        let mut late = Frame::new(120, 120);
        late.draw_ripple(60.0, 60.0, 0.8, &style);
        let late_pixels = late.data.chunks_exact(4).filter(|p| p[3] > 0).count();

        assert!(early_pixels > 0, "a ripple should draw");
        assert!(
            late_pixels > early_pixels,
            "an expanding ring should cover more pixels: {late_pixels} vs {early_pixels}"
        );
    }

    #[test]
    fn a_finished_ripple_draws_nothing() {
        let mut frame = Frame::new(120, 120);
        frame.draw_ripple(60.0, 60.0, 1.0, &CursorStyle::default());
        assert!(frame.is_empty());
    }

    #[test]
    fn the_ripple_is_a_ring_not_a_disc() {
        let style = CursorStyle::default();
        let mut frame = Frame::new(200, 200);
        frame.draw_ripple(100.0, 100.0, 0.5, &style);

        // Dead centre is inside the ring's hole, not on the ring itself.
        assert_eq!(frame.get(100, 100)[3], 0, "centre should be clear");
        // Somewhere out on the ring, something is painted.
        let painted = frame.data.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(painted > 20, "expected a ring, got {painted} pixels");
    }

    #[test]
    fn a_larger_style_draws_more_pixels() {
        let small = CursorStyle {
            scale: 1.0,
            ..CursorStyle::default()
        };
        let large = CursorStyle {
            scale: 3.0,
            ..CursorStyle::default()
        };

        let count = |style: &CursorStyle| {
            let mut frame = Frame::new(400, 400);
            frame.draw_cursor(200.0, 200.0, style);
            frame.data.chunks_exact(4).filter(|p| p[3] > 0).count()
        };

        assert!(count(&large) > count(&small));
    }

    #[test]
    fn the_polygon_helper_agrees_with_itself() {
        let points = arrow_points(28.0, 40.0);
        // The tip is inside; a point well outside the box is not.
        assert!(point_in_polygon(1.0, 1.0, &points));
        assert!(!point_in_polygon(100.0, 100.0, &points));
        assert!(!point_in_polygon(-5.0, 20.0, &points));
    }

    #[test]
    fn distance_to_a_segment_handles_degenerate_input() {
        // A zero-length segment has no direction, so it degrades to a point.
        let degenerate = distance_to_segment(1.0, 1.0, 5.0, 5.0, 5.0, 5.0);
        assert!(degenerate.is_finite());
        assert!((degenerate - (16.0_f64 + 16.0).sqrt()).abs() < 1e-9);

        // A point on the segment is zero away from it.
        assert!((distance_to_segment(0.0, 0.0, 0.0, 0.0, 10.0, 0.0) - 0.0).abs() < 1e-9);
        assert!((distance_to_segment(5.0, 0.0, 0.0, 0.0, 10.0, 0.0) - 0.0).abs() < 1e-9);

        // The projection is clamped to the segment's ends, so a point past either
        // end is measured from that endpoint rather than from the infinite line.
        assert!((distance_to_segment(15.0, 0.0, 0.0, 0.0, 10.0, 0.0) - 5.0).abs() < 1e-9);
        assert!((distance_to_segment(-5.0, 0.0, 0.0, 0.0, 10.0, 0.0) - 5.0).abs() < 1e-9);
    }
}
