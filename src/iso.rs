use macroquad::prelude::*;
use std::f32::consts::TAU;

pub const TW: f32 = 64.0;
pub const TH: f32 = 32.0;

#[inline]
pub fn world_to_screen(w: Vec2) -> Vec2 {
    vec2((w.x - w.y) * TW * 0.5, (w.x + w.y) * TH * 0.5)
}

#[inline]
pub fn screen_to_world(s: Vec2) -> Vec2 {
    vec2(s.x / TW + s.y / TH, s.y / TH - s.x / TW)
}

#[inline]
pub fn y_sort_key(world_pos: Vec2, z: f32) -> f32 {
    const K: f32 = 1000.0;
    world_pos.x + world_pos.y - z * K
}

#[inline]
pub fn world_angle_to_screen_angle(world_angle: f32) -> f32 {
    let world_dir = vec2(world_angle.cos(), world_angle.sin());
    let screen_dir = world_to_screen(world_dir);
    screen_dir.y.atan2(screen_dir.x)
}

// Inverse of world_angle_to_screen_angle. Solves the 2x2 linear system
// from world_to_screen for a unit direction, so that turning applied in
// screen space stays visually uniform despite the 2:1 iso stretch.
#[inline]
pub fn screen_angle_to_world_angle(screen_angle: f32) -> f32 {
    let sx = screen_angle.cos();
    let sy = screen_angle.sin();
    let wx = sx / TW + sy / TH;
    let wy = sy / TH - sx / TW;
    wy.atan2(wx)
}

#[inline]
pub fn angle_to_frame(angle: f32, frames: usize, frame_zero_angle: f32) -> usize {
    let step = TAU / frames as f32;
    let frame = ((angle - frame_zero_angle).rem_euclid(TAU) / step).round() as usize;
    frame % frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_to_world_inverts_isometric_projection() {
        let world = vec2(7.25, -3.5);
        let restored = screen_to_world(world_to_screen(world));
        assert!((restored - world).length() < 1e-5);
    }
}
