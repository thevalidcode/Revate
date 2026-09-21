//! Spotlight effect rendering.
//! Implements a focused spotlight effect during zoom operations.
//! TODO: Implement spotlight effect for zoom transitions.

pub fn render_spotlight(
    frame: &mut Frame,
    center: &Point,
    radius: f64,
    opacity: f32,
) -> Result<(), anyhow::Error> {
    todo!("Implement spotlight rendering")
}

pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub struct Frame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
