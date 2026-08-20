use std::f32::consts::FRAC_PI_4;

use macroquad::prelude::*;

use crate::iso::{angle_to_frame, world_angle_to_screen_angle, world_to_screen};
use crate::world::Rock;

// ── Shadow generation parameters ─────────────────────────────────────────────
// All sizes in "shadow pixels" (= full-res / SHADOW_SCALE) unless noted.

const SHADOW_SCALE: usize = 4; // downsample factor — keeps generation fast
const SHADOW_PAD: usize = 5; // extra cells per side so blur doesn't clip at the sprite edge
const SHADOW_BLUR_R: i32 = 4; // default box-blur radius for rocks and ships
const ROCKET_SHADOW_BLUR_R: i32 = 1; // rockets need a tighter mask at their small source size
const SHADOW_ALPHA: f32 = 0.55;
// Padding expressed in full-res pixels (used to derive draw-space offsets).
const SHADOW_PAD_FULL: f32 = (SHADOW_PAD * SHADOW_SCALE) as f32; // = 20.0

// ── Rock sprites ──────────────────────────────────────────────────────────────

const ROCK_SHEET_BYTES: &[u8] = include_bytes!("../assets/rock_sheet_iso_4x4.png");
const ROCK_VARIANTS: usize = 16;
const ROCK_COLS: usize = 4;
const ROCK_DRAW_SCALE: f32 = 120.0;
// Shadow cast direction (top-left), as a fraction of the draw size.
const ROCK_SHADOW_DX: f32 = -0.075;
const ROCK_SHADOW_DY: f32 = -0.1;

pub struct RockSprites {
    sheet: Texture2D,
    cell_w: f32,
    cell_h: f32,
    // Single atlas for all rock shadows — avoids per-rock texture switches.
    shadow_atlas: Texture2D,
    shadow_cell_w: usize, // atlas cell width  in shadow pixels
    shadow_cell_h: usize, // atlas cell height in shadow pixels
}

impl RockSprites {
    pub fn load() -> Self {
        let img = Image::from_file_with_format(ROCK_SHEET_BYTES, Some(ImageFormat::Png))
            .expect("failed to decode rock sprite sheet");
        let sheet = Texture2D::from_image(&img);
        sheet.set_filter(FilterMode::Linear);
        let cell_w = img.width as f32 / ROCK_COLS as f32;
        let cell_h = img.height as f32 / (ROCK_VARIANTS / ROCK_COLS) as f32;

        let scw = cell_w as usize / SHADOW_SCALE + SHADOW_PAD * 2;
        let sch = cell_h as usize / SHADOW_SCALE + SHADOW_PAD * 2;
        let atlas_rows = ROCK_VARIANTS / ROCK_COLS;
        let mut atlas_img = Image::gen_image_color(
            (ROCK_COLS * scw) as u16,
            (atlas_rows * sch) as u16,
            Color::new(0.0, 0.0, 0.0, 0.0),
        );
        for v in 0..ROCK_VARIANTS {
            let cx = (v % ROCK_COLS) as f32 * cell_w;
            let cy = (v / ROCK_COLS) as f32 * cell_h;
            let cell_img = img.sub_image(Rect::new(cx, cy, cell_w, cell_h));
            let shadow_cell = make_shadow_cell(&cell_img, SHADOW_PAD, SHADOW_BLUR_R);
            blit_into(
                &shadow_cell,
                &mut atlas_img,
                (v % ROCK_COLS) * scw,
                (v / ROCK_COLS) * sch,
            );
        }
        let shadow_atlas = Texture2D::from_image(&atlas_img);
        shadow_atlas.set_filter(FilterMode::Linear);

        Self {
            sheet,
            cell_w,
            cell_h,
            shadow_atlas,
            shadow_cell_w: scw,
            shadow_cell_h: sch,
        }
    }

    pub fn draw_shadow(&self, rock: &Rock) {
        let s = world_to_screen(rock.pos);
        let size = rock.radius * ROCK_DRAW_SCALE;
        let v = rock.variant as usize % ROCK_VARIANTS;

        let pad_x = SHADOW_PAD_FULL * (size / self.cell_w);
        let pad_y = SHADOW_PAD_FULL * (size / self.cell_h);
        let shadow_src = Rect::new(
            ((v % ROCK_COLS) * self.shadow_cell_w) as f32,
            ((v / ROCK_COLS) * self.shadow_cell_h) as f32,
            self.shadow_cell_w as f32,
            self.shadow_cell_h as f32,
        );
        draw_texture_ex(
            &self.shadow_atlas,
            s.x - size * 0.5 - pad_x + size * ROCK_SHADOW_DX,
            s.y - size * 0.5 - pad_y + size * ROCK_SHADOW_DY,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(size + 2.0 * pad_x, size + 2.0 * pad_y)),
                source: Some(shadow_src),
                ..Default::default()
            },
        );
    }

    pub fn draw(&self, rock: &Rock) {
        let s = world_to_screen(rock.pos);
        let size = rock.radius * ROCK_DRAW_SCALE;
        let v = rock.variant as usize % ROCK_VARIANTS;
        let src = Rect::new(
            (v % ROCK_COLS) as f32 * self.cell_w,
            (v / ROCK_COLS) as f32 * self.cell_h,
            self.cell_w,
            self.cell_h,
        );
        draw_texture_ex(
            &self.sheet,
            s.x - size * 0.5,
            s.y - size * 0.5,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(size, size)),
                source: Some(src),
                ..Default::default()
            },
        );
    }
}

// ── Ship sprites ──────────────────────────────────────────────────────────────

const SHIP_SHEET_BYTES: &[u8] = include_bytes!("../assets/ship_strip.png");
const SHIP_FRAMES: usize = 128;
const SHIP_FRAME_SIZE: f32 = 200.0;
const SHIP_DRAW_SIZE: f32 = 74.0;
// Shadow cast direction in draw pixels (top-left, matching the sprite light source).
const SHIP_SHADOW_DX: f32 = -4.5;
const SHIP_SHADOW_DY: f32 = -6.0;

// Sprite atlas layout: 256 frames (128 rotations × 2 player rows) repacked from
// the wide source strip into a 16-column grid (→ 16 rows, 3200×3200 px). One
// texture lets every ship draw share a batch and stays under the GPU max texture
// size that a 25600-px-wide strip would exceed.
const SHIP_ATLAS_COLS: usize = 16;
// Shadow atlas layout: 16 columns × 8 rows = 128 frames.
const SHIP_SHADOW_CELL: usize = 200 / SHADOW_SCALE + SHADOW_PAD * 2; // = 60 shadow px
const SHIP_SHADOW_ATLAS_COLS: usize = 16;
const SHIP_SHADOW_ATLAS_ROWS: usize = 8;

pub struct ShipSprites {
    // Single atlas holding all 256 oriented frames — one texture, sampled per
    // frame via a source Rect (same pattern as the rock/rocket sheets).
    atlas: Texture2D,
    // Single atlas for all 128 rotation shadows — batches cleanly with one texture switch.
    shadow_atlas: Texture2D,
}

impl ShipSprites {
    pub fn load() -> Self {
        let sheet = Image::from_file_with_format(SHIP_SHEET_BYTES, Some(ImageFormat::Png))
            .expect("failed to decode ship sprite sheet");
        let expected_width = SHIP_FRAMES as u16 * SHIP_FRAME_SIZE as u16;
        let expected_height = 2 * SHIP_FRAME_SIZE as u16;
        assert_eq!(sheet.width, expected_width, "unexpected ship sheet width");
        assert_eq!(
            sheet.height, expected_height,
            "unexpected ship sheet height"
        );

        // Sprite atlas: all 256 frames repacked from the wide strip into a square
        // grid. Shadow atlas: built from row 0 only (the silhouette is identical
        // for both player tints). Both are filled in the same sub-image loop.
        let cell = SHIP_FRAME_SIZE as usize;
        let atlas_rows = (SHIP_FRAMES * 2).div_ceil(SHIP_ATLAS_COLS);
        let mut atlas_img = Image::gen_image_color(
            (SHIP_ATLAS_COLS * cell) as u16,
            (atlas_rows * cell) as u16,
            Color::new(0.0, 0.0, 0.0, 0.0),
        );
        let mut shadow_atlas_img = Image::gen_image_color(
            (SHIP_SHADOW_ATLAS_COLS * SHIP_SHADOW_CELL) as u16,
            (SHIP_SHADOW_ATLAS_ROWS * SHIP_SHADOW_CELL) as u16,
            Color::new(0.0, 0.0, 0.0, 0.0),
        );

        for row in 0..2usize {
            for frame in 0..SHIP_FRAMES {
                let image = sheet.sub_image(Rect::new(
                    frame as f32 * SHIP_FRAME_SIZE,
                    row as f32 * SHIP_FRAME_SIZE,
                    SHIP_FRAME_SIZE,
                    SHIP_FRAME_SIZE,
                ));
                let ai = row * SHIP_FRAMES + frame;
                blit_into(
                    &image,
                    &mut atlas_img,
                    (ai % SHIP_ATLAS_COLS) * cell,
                    (ai / SHIP_ATLAS_COLS) * cell,
                );
                if row == 0 {
                    let shadow_cell = make_shadow_cell(&image, SHADOW_PAD, SHADOW_BLUR_R);
                    blit_into(
                        &shadow_cell,
                        &mut shadow_atlas_img,
                        (frame % SHIP_SHADOW_ATLAS_COLS) * SHIP_SHADOW_CELL,
                        (frame / SHIP_SHADOW_ATLAS_COLS) * SHIP_SHADOW_CELL,
                    );
                }
            }
        }

        let atlas = Texture2D::from_image(&atlas_img);
        atlas.set_filter(FilterMode::Linear);
        let shadow_atlas = Texture2D::from_image(&shadow_atlas_img);
        shadow_atlas.set_filter(FilterMode::Linear);

        Self {
            atlas,
            shadow_atlas,
        }
    }

    pub fn draw_shadow(&self, world_angle: f32, pos: Vec2) {
        let screen_angle = world_angle_to_screen_angle(world_angle);
        let frame = angle_to_frame(screen_angle, SHIP_FRAMES, FRAC_PI_4 * 3.0);

        let bx = pos.x - SHIP_DRAW_SIZE * 0.5;
        let by = pos.y - SHIP_DRAW_SIZE * 0.5;
        let draw_scale = SHIP_DRAW_SIZE / SHIP_FRAME_SIZE;
        let pad_draw = SHADOW_PAD_FULL * draw_scale;
        let shadow_size = SHIP_SHADOW_CELL as f32 * SHADOW_SCALE as f32 * draw_scale;
        let ax = (frame % SHIP_SHADOW_ATLAS_COLS) * SHIP_SHADOW_CELL;
        let ay = (frame / SHIP_SHADOW_ATLAS_COLS) * SHIP_SHADOW_CELL;
        let shadow_src = Rect::new(
            ax as f32,
            ay as f32,
            SHIP_SHADOW_CELL as f32,
            SHIP_SHADOW_CELL as f32,
        );
        draw_texture_ex(
            &self.shadow_atlas,
            bx - pad_draw + SHIP_SHADOW_DX,
            by - pad_draw + SHIP_SHADOW_DY,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(shadow_size, shadow_size)),
                source: Some(shadow_src),
                ..Default::default()
            },
        );
    }

    pub fn draw(&self, world_angle: f32, owner: usize, pos: Vec2, tint: Color) {
        let row = if owner == 0 { 1 } else { 0 };
        let screen_angle = world_angle_to_screen_angle(world_angle);
        let frame = angle_to_frame(screen_angle, SHIP_FRAMES, FRAC_PI_4 * 3.0);
        let ai = row * SHIP_FRAMES + frame;
        let src = Rect::new(
            (ai % SHIP_ATLAS_COLS) as f32 * SHIP_FRAME_SIZE,
            (ai / SHIP_ATLAS_COLS) as f32 * SHIP_FRAME_SIZE,
            SHIP_FRAME_SIZE,
            SHIP_FRAME_SIZE,
        );
        draw_texture_ex(
            &self.atlas,
            pos.x - SHIP_DRAW_SIZE * 0.5,
            pos.y - SHIP_DRAW_SIZE * 0.5,
            tint,
            DrawTextureParams {
                dest_size: Some(vec2(SHIP_DRAW_SIZE, SHIP_DRAW_SIZE)),
                source: Some(src),
                ..Default::default()
            },
        );
    }
}

// ── Rocket sprites ──────────────────────────────────────────────────────────

const ROCKET_SHEET_BYTES: &[u8] = include_bytes!("../assets/rocket_strip.png");
const ROCKET_FRAMES: usize = 64;
const ROCKET_FRAME_SIZE: f32 = 32.0;
const ROCKET_DRAW_SIZE: f32 = 44.0;
const ROCKET_SHADOW_CELL: usize = 32 / SHADOW_SCALE + SHADOW_PAD * 2;
const ROCKET_SHADOW_ATLAS_COLS: usize = 8;
const ROCKET_SHADOW_DX: f32 = -4.5;
const ROCKET_SHADOW_DY: f32 = -6.0;

pub struct RocketSprites {
    // One texture, sampled per-frame via a source Rect (single horizontal strip).
    sheet: Texture2D,
    shadow_atlas: Texture2D,
}

impl RocketSprites {
    pub fn load() -> Self {
        let img = Image::from_file_with_format(ROCKET_SHEET_BYTES, Some(ImageFormat::Png))
            .expect("failed to decode rocket sprite strip");
        let expected_width = ROCKET_FRAMES as u16 * ROCKET_FRAME_SIZE as u16;
        assert_eq!(img.width, expected_width, "unexpected rocket strip width");
        assert_eq!(
            img.height, ROCKET_FRAME_SIZE as u16,
            "unexpected rocket strip height"
        );
        let sheet = Texture2D::from_image(&img);
        sheet.set_filter(FilterMode::Linear);

        let shadow_rows = ROCKET_FRAMES.div_ceil(ROCKET_SHADOW_ATLAS_COLS);
        let mut shadow_atlas_img = Image::gen_image_color(
            (ROCKET_SHADOW_ATLAS_COLS * ROCKET_SHADOW_CELL) as u16,
            (shadow_rows * ROCKET_SHADOW_CELL) as u16,
            Color::new(0.0, 0.0, 0.0, 0.0),
        );
        for frame in 0..ROCKET_FRAMES {
            let image = img.sub_image(Rect::new(
                frame as f32 * ROCKET_FRAME_SIZE,
                0.0,
                ROCKET_FRAME_SIZE,
                ROCKET_FRAME_SIZE,
            ));
            let shadow_cell = make_shadow_cell(&image, SHADOW_PAD, ROCKET_SHADOW_BLUR_R);
            blit_into(
                &shadow_cell,
                &mut shadow_atlas_img,
                (frame % ROCKET_SHADOW_ATLAS_COLS) * ROCKET_SHADOW_CELL,
                (frame / ROCKET_SHADOW_ATLAS_COLS) * ROCKET_SHADOW_CELL,
            );
        }
        let shadow_atlas = Texture2D::from_image(&shadow_atlas_img);
        shadow_atlas.set_filter(FilterMode::Linear);

        Self {
            sheet,
            shadow_atlas,
        }
    }

    pub fn draw_shadow(&self, world_angle: f32, pos: Vec2) {
        let screen_angle = world_angle_to_screen_angle(world_angle);
        let frame = angle_to_frame(screen_angle, ROCKET_FRAMES, FRAC_PI_4 * 3.0);
        let draw_scale = ROCKET_DRAW_SIZE / ROCKET_FRAME_SIZE;
        let pad_draw = SHADOW_PAD_FULL * draw_scale;
        let shadow_size = ROCKET_SHADOW_CELL as f32 * SHADOW_SCALE as f32 * draw_scale;
        let ax = (frame % ROCKET_SHADOW_ATLAS_COLS) * ROCKET_SHADOW_CELL;
        let ay = (frame / ROCKET_SHADOW_ATLAS_COLS) * ROCKET_SHADOW_CELL;
        let shadow_src = Rect::new(
            ax as f32,
            ay as f32,
            ROCKET_SHADOW_CELL as f32,
            ROCKET_SHADOW_CELL as f32,
        );
        let s = world_to_screen(pos);
        draw_texture_ex(
            &self.shadow_atlas,
            s.x - ROCKET_DRAW_SIZE * 0.5 - pad_draw + ROCKET_SHADOW_DX,
            s.y - ROCKET_DRAW_SIZE * 0.5 - pad_draw + ROCKET_SHADOW_DY,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(shadow_size, shadow_size)),
                source: Some(shadow_src),
                ..Default::default()
            },
        );
    }

    pub fn draw(&self, world_angle: f32, pos: Vec2) {
        // Same iso convention as the ship strip: convert to screen angle first,
        // then index the frame (frame 0 at 3π/4, matching the strip's layout).
        let screen_angle = world_angle_to_screen_angle(world_angle);
        let frame = angle_to_frame(screen_angle, ROCKET_FRAMES, FRAC_PI_4 * 3.0);
        let src = Rect::new(
            frame as f32 * ROCKET_FRAME_SIZE,
            0.0,
            ROCKET_FRAME_SIZE,
            ROCKET_FRAME_SIZE,
        );
        let s = world_to_screen(pos);
        draw_texture_ex(
            &self.sheet,
            s.x - ROCKET_DRAW_SIZE * 0.5,
            s.y - ROCKET_DRAW_SIZE * 0.5,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(ROCKET_DRAW_SIZE, ROCKET_DRAW_SIZE)),
                source: Some(src),
                ..Default::default()
            },
        );
    }

    pub fn draw_propellant_glow(&self, world_angle: f32, pos: Vec2) {
        let screen_angle = world_angle_to_screen_angle(world_angle);
        let forward = vec2(screen_angle.cos(), screen_angle.sin());
        let tail = world_to_screen(pos) - forward * (ROCKET_DRAW_SIZE * 0.32);
        let pulse = 0.9 + 0.1 * (get_time() as f32 * 18.0).sin();

        // Broad, faint layers lift the surrounding terrain without leaving a
        // hard-edged disc around the compact propellant flare.
        draw_circle(
            tail.x,
            tail.y,
            34.0 * pulse,
            Color::new(0.08, 0.3, 1.0, 0.045),
        );
        draw_circle(
            tail.x,
            tail.y,
            23.0 * pulse,
            Color::new(0.06, 0.4, 1.0, 0.085),
        );
        draw_circle(
            tail.x,
            tail.y,
            10.0 * pulse,
            Color::new(0.05, 0.35, 1.0, 0.16),
        );
        draw_circle(
            tail.x,
            tail.y,
            5.5 * pulse,
            Color::new(0.05, 0.65, 1.0, 0.5),
        );
        draw_circle(
            tail.x,
            tail.y,
            2.3 * pulse,
            Color::new(0.65, 0.95, 1.0, 0.95),
        );
    }
}

// ── Explosion sprites ───────────────────────────────────────────────────────

const EXPL_SHEET_BYTES: &[u8] = include_bytes!("../assets/explosion_sheet_iso_10x6.png");
const EXPL_COLS: usize = 10;
const EXPL_ROWS: usize = 6;
const EXPL_FRAMES: usize = EXPL_COLS * EXPL_ROWS;
// Drawn size on screen — roughly three iso tiles across, so the burst reads
// at the same scale as the procedural ring it sits on top of.
const EXPL_DRAW_SIZE: f32 = 200.0;

pub struct ExplosionSprites {
    // One texture, sampled per-frame via a source Rect (no per-frame splitting).
    sheet: Texture2D,
    cell_w: f32,
    cell_h: f32,
}

impl ExplosionSprites {
    pub fn load() -> Self {
        let img = Image::from_file_with_format(EXPL_SHEET_BYTES, Some(ImageFormat::Png))
            .expect("failed to decode explosion sprite sheet");
        let sheet = Texture2D::from_image(&img);
        sheet.set_filter(FilterMode::Linear);
        Self {
            sheet,
            cell_w: img.width as f32 / EXPL_COLS as f32,
            cell_h: img.height as f32 / EXPL_ROWS as f32,
        }
    }

    /// Draws the animated burst centered on world `pos`. `t` is the animation
    /// progress in `0.0..=1.0` (0 = ignition, 1 = fully dissipated). `scale`
    /// multiplies the drawn size — 1.0 for a full-size blast, smaller for a
    /// scaled-down effect (e.g. rock destruction).
    pub fn draw(&self, pos: Vec2, t: f32, scale: f32) {
        let frame = ((t.clamp(0.0, 1.0) * EXPL_FRAMES as f32) as usize).min(EXPL_FRAMES - 1);
        let src = Rect::new(
            (frame % EXPL_COLS) as f32 * self.cell_w,
            (frame / EXPL_COLS) as f32 * self.cell_h,
            self.cell_w,
            self.cell_h,
        );
        let size = EXPL_DRAW_SIZE * scale;
        let s = world_to_screen(pos);
        draw_texture_ex(
            &self.sheet,
            s.x - size * 0.5,
            s.y - size * 0.5,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(size, size)),
                source: Some(src),
                ..Default::default()
            },
        );
    }
}

// ── Shadow helpers ────────────────────────────────────────────────────────────

// Generates a padded shadow cell Image from the alpha channel of `src`.
// Output size: (src.width/SHADOW_SCALE + 2*pad) × (src.height/SHADOW_SCALE + 2*pad).
fn make_shadow_cell(src: &Image, pad: usize, blur_radius: i32) -> Image {
    let sw = src.width as usize;
    let sh = src.height as usize;
    let w = sw / SHADOW_SCALE + pad * 2;
    let h = sh / SHADOW_SCALE + pad * 2;

    // Downsample: box-average SHADOW_SCALE×SHADOW_SCALE source blocks.
    // Pixels in the padding region map outside the source and contribute alpha=0.
    let mut alpha = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0f32;
            for dy in 0..SHADOW_SCALE {
                for dx in 0..SHADOW_SCALE {
                    let px = (x as i32 - pad as i32) * SHADOW_SCALE as i32 + dx as i32;
                    let py = (y as i32 - pad as i32) * SHADOW_SCALE as i32 + dy as i32;
                    if px >= 0 && px < sw as i32 && py >= 0 && py < sh as i32 {
                        sum += src.get_pixel(px as u32, py as u32).a;
                    }
                }
            }
            alpha[y * w + x] = sum / (SHADOW_SCALE * SHADOW_SCALE) as f32;
        }
    }

    // Separable box blur — horizontal pass.
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let x0 = (x - blur_radius).max(0);
            let x1 = (x + blur_radius).min(w as i32 - 1);
            let mut s = 0.0f32;
            for nx in x0..=x1 {
                s += alpha[y as usize * w + nx as usize];
            }
            tmp[y as usize * w + x as usize] = s / (x1 - x0 + 1) as f32;
        }
    }

    // Separable box blur — vertical pass.
    let mut blurred = vec![0.0f32; w * h];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let y0 = (y - blur_radius).max(0);
            let y1 = (y + blur_radius).min(h as i32 - 1);
            let mut s = 0.0f32;
            for ny in y0..=y1 {
                s += tmp[ny as usize * w + x as usize];
            }
            blurred[y as usize * w + x as usize] = s / (y1 - y0 + 1) as f32;
        }
    }

    let mut img = Image::gen_image_color(w as u16, h as u16, Color::new(0.0, 0.0, 0.0, 0.0));
    for y in 0..h {
        for x in 0..w {
            let a = (blurred[y * w + x] * SHADOW_ALPHA).clamp(0.0, 1.0);
            img.set_pixel(x as u32, y as u32, Color::new(0.0, 0.0, 0.0, a));
        }
    }
    img
}

// Copies `src` into `dst` at pixel offset (dx, dy). Both images are RGBA8, so
// each source row is one contiguous slice copy — fast enough to repack the full
// 256-frame ship atlas (~10 M px) at load without a per-pixel loop.
fn blit_into(src: &Image, dst: &mut Image, dx: usize, dy: usize) {
    let sw = src.width as usize;
    let sh = src.height as usize;
    let dw = dst.width as usize;
    let row_bytes = sw * 4;
    for y in 0..sh {
        let s0 = y * row_bytes;
        let d0 = ((dy + y) * dw + dx) * 4;
        dst.bytes[d0..d0 + row_bytes].copy_from_slice(&src.bytes[s0..s0 + row_bytes]);
    }
}
