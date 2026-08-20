use std::f32::consts::TAU;

use ::rand::{RngExt, SeedableRng};
use macroquad::prelude::*;
use rand_xoshiro::Xoshiro256PlusPlus;

use crate::iso::{TH, TW};
use crate::world::World;

// Maximum rocks uploaded to the background shader — must match MAX_ROCKS in
// assets/background.frag.
pub const MAX_ROCKS: usize = 64;

const MAX_BG_ROCKETS: usize = 4;

// Reach beyond the visible diamond's corner: a rock just off-viewport still
// perturbs the sand inside it out to about this distance.
const GATHER_MARGIN: f32 = 3.0;

// World-space radius around the viewport centre within which a rock can perturb
// a visible fragment. Derived from the viewport rather than fixed, because the
// single-screen viewport is twice as wide as a split-screen half (≈ 13 tiles of
// diamond reach at 480×540, ≈ 16 at 960×540).
fn gather_radius(view_size: Vec2) -> f32 {
    // Corner of the viewport through the inverse iso projection: with
    // wx = a + b and wy = b − a, its distance is sqrt(2·(a² + b²)).
    let a = view_size.x * 0.5 / TW;
    let b = view_size.y * 0.5 / TH;
    (2.0 * (a * a + b * b)).sqrt() + GATHER_MARGIN
}

#[derive(Clone, Copy)]
pub struct BiomeParams {
    pub palette_a: Vec3,
    pub palette_b: Vec3,
    pub palette_c: Vec3,
    pub wind_dir: Vec2,
    pub dune_stretch: f32,
    pub warp_amp: f32,
    pub octaves: i32,
    pub seed: u32,
    pub terrain_offset: Vec2,
    pub flow_speed: f32,
    pub ridge_exponent: f32,
}

pub struct BackgroundView {
    pub world_pos: Vec2,
    pub target: Vec2,
    pub size: Vec2,
}

pub struct BackgroundObstacles<'a> {
    pub ships: &'a [Vec3],
    pub ship_dirs: &'a [Vec2],
    pub ship_thrust: &'a [f32],
    pub rockets: &'a [Vec3],
    pub rocket_dirs: &'a [Vec2],
}

// Curated palettes — each entry is (shadow/horizon, lit/zenith, rim-glow or
// deep-water). Hand-tuned so every random pick reads cleanly.
const DESERT_PALETTES: [[Vec3; 3]; 3] = [
    [
        vec3(0.72, 0.54, 0.32),
        vec3(1.00, 0.90, 0.62),
        vec3(1.00, 0.65, 0.28),
    ],
    [
        vec3(0.55, 0.30, 0.20),
        vec3(0.95, 0.62, 0.38),
        vec3(1.00, 0.46, 0.22),
    ],
    [
        vec3(0.66, 0.62, 0.55),
        vec3(0.98, 0.96, 0.88),
        vec3(0.92, 0.78, 0.58),
    ],
];

impl BiomeParams {
    pub fn desert() -> Self {
        Self {
            palette_a: vec3(0.72, 0.54, 0.32), // shadow sand
            palette_b: vec3(1.00, 0.90, 0.62), // lit sand
            palette_c: vec3(1.00, 0.65, 0.28), // rim glow
            wind_dir: vec2(0.85, 0.53),
            dune_stretch: 0.28,
            warp_amp: 0.35,
            octaves: 5,
            seed: 12345,
            terrain_offset: Vec2::ZERO,
            flow_speed: 1.0,
            ridge_exponent: 6.0,
        }
    }

    /// Fully randomised desert background look for a fresh game.
    pub fn random(seed: u64) -> Self {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);

        let mut p = Self::desert();

        let pal = DESERT_PALETTES[rng.random_range(0..DESERT_PALETTES.len())];
        p.palette_a = pal[0];
        p.palette_b = pal[1];
        p.palette_c = pal[2];

        let angle: f32 = rng.random_range(0.0..TAU);
        p.wind_dir = vec2(angle.cos(), angle.sin());
        p.dune_stretch = rng.random_range(0.18..0.42);
        p.warp_amp = rng.random_range(0.16..0.42);
        p.octaves = 4 + rng.random_range(0i32..3);
        p.seed = rng.random::<u32>();
        p.terrain_offset = vec2(
            rng.random_range(-600.0..600.0),
            rng.random_range(-600.0..600.0),
        );
        p.flow_speed = rng.random_range(0.80_f32..1.20);
        p.ridge_exponent = rng.random_range(0.80_f32..6.0);
        p
    }
}

pub struct Background {
    material: Material,
}

const VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
varying lowp vec2 uv;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    uv = texcoord;
}"#;

const FS: &str = include_str!("../assets/background.frag");

impl Background {
    pub fn new() -> Self {
        let material = load_material(
            ShaderSource::Glsl {
                vertex: VS,
                fragment: FS,
            },
            MaterialParams {
                uniforms: vec![
                    UniformDesc::new("u_resolution", UniformType::Float2),
                    UniformDesc::new("u_time", UniformType::Float1),
                    UniformDesc::new("u_seed", UniformType::Int1),
                    UniformDesc::new("u_cam_origin", UniformType::Float2),
                    UniformDesc::new("u_pixels_per_unit", UniformType::Float1),
                    UniformDesc::new("u_wind_dir", UniformType::Float2),
                    UniformDesc::new("u_dune_stretch", UniformType::Float1),
                    UniformDesc::new("u_warp_amp", UniformType::Float1),
                    UniformDesc::new("u_sun_dir", UniformType::Float3),
                    UniformDesc::new("u_palette_a", UniformType::Float3),
                    UniformDesc::new("u_palette_b", UniformType::Float3),
                    UniformDesc::new("u_palette_c", UniformType::Float3),
                    UniformDesc::new("u_octaves", UniformType::Int1),
                    UniformDesc::new("u_terrain_offset", UniformType::Float2),
                    UniformDesc::new("u_flow_speed", UniformType::Float1),
                    UniformDesc::new("u_ridge_exponent", UniformType::Float1),
                    UniformDesc::new("u_rocks", UniformType::Float3).array(MAX_ROCKS),
                    UniformDesc::new("u_rock_count", UniformType::Int1),
                    UniformDesc::new("u_ships", UniformType::Float3).array(2),
                    UniformDesc::new("u_ship_dir", UniformType::Float2).array(2),
                    UniformDesc::new("u_ship_thrust", UniformType::Float1).array(2),
                    UniformDesc::new("u_ship_count", UniformType::Int1),
                    UniformDesc::new("u_rockets", UniformType::Float3).array(MAX_BG_ROCKETS),
                    UniformDesc::new("u_rocket_dir", UniformType::Float2).array(MAX_BG_ROCKETS),
                    UniformDesc::new("u_rocket_count", UniformType::Int1),
                ],
                ..Default::default()
            },
        )
        .expect("background shader");

        Self { material }
    }

    /// Draw the procedural background into the currently-active render target.
    ///
    /// `view` describes the camera position, target, and render-target size.
    /// `world` supplies nearby rocks that perturb the surface flow.
    /// `obstacles` supplies live ships and rockets plus their exhaust directions.
    pub fn draw(
        &self,
        p: &BiomeParams,
        view: BackgroundView,
        world: &World,
        obstacles: BackgroundObstacles<'_>,
    ) {
        let t = (get_time() as f32) % 3600.0;
        let vw = view.size.x;
        let vh = view.size.y;

        // Collect rocks whose perturbation can reach the visible area, nearest
        // first so the closest MAX_ROCKS survive in dense rock fields. The wider
        // single-screen viewport gathers from a larger area against the same
        // MAX_ROCKS budget, so its outermost rocks are the ones that lose their
        // sand perturbation first.
        let gather = gather_radius(view.size);
        let mut rocks: Vec<Vec3> = world
            .rocks_near(view.world_pos)
            .filter(|r| (r.pos - view.world_pos).length_squared() <= gather * gather)
            .map(|r| vec3(r.pos.x, r.pos.y, r.radius))
            .collect();
        if rocks.len() > MAX_ROCKS {
            rocks.sort_unstable_by(|a, b| {
                let da = (a.truncate() - view.world_pos).length_squared();
                let db = (b.truncate() - view.world_pos).length_squared();
                da.total_cmp(&db)
            });
            rocks.truncate(MAX_ROCKS);
        }
        let rock_count = rocks.len();
        let mut rock_buf = [Vec3::ZERO; MAX_ROCKS];
        rock_buf[..rock_count].copy_from_slice(&rocks);

        let ship_count = obstacles.ships.len().min(2);
        let mut ship_buf = [Vec3::ZERO; 2];
        ship_buf[..ship_count].copy_from_slice(&obstacles.ships[..ship_count]);
        let mut ship_dir_buf = [Vec2::ZERO; 2];
        ship_dir_buf[..ship_count].copy_from_slice(&obstacles.ship_dirs[..ship_count]);
        let mut ship_thrust_buf = [0.0_f32; 2];
        ship_thrust_buf[..ship_count].copy_from_slice(&obstacles.ship_thrust[..ship_count]);

        let rocket_count = obstacles.rockets.len().min(MAX_BG_ROCKETS);
        let mut rocket_buf = [Vec3::ZERO; MAX_BG_ROCKETS];
        rocket_buf[..rocket_count].copy_from_slice(&obstacles.rockets[..rocket_count]);
        let mut rocket_dir_buf = [Vec2::ZERO; MAX_BG_ROCKETS];
        rocket_dir_buf[..rocket_count].copy_from_slice(&obstacles.rocket_dirs[..rocket_count]);

        self.material.set_uniform("u_resolution", vec2(vw, vh));
        self.material.set_uniform("u_time", t);
        self.material.set_uniform("u_seed", p.seed as i32);
        self.material
            .set_uniform("u_terrain_offset", p.terrain_offset);
        self.material.set_uniform("u_flow_speed", p.flow_speed);
        self.material
            .set_uniform("u_ridge_exponent", p.ridge_exponent);
        self.material.set_uniform("u_cam_origin", view.world_pos);
        self.material.set_uniform("u_pixels_per_unit", 1.0_f32);
        self.material.set_uniform("u_wind_dir", p.wind_dir);
        self.material.set_uniform("u_dune_stretch", p.dune_stretch);
        self.material.set_uniform("u_warp_amp", p.warp_amp);
        // Light from screen-top + near the camera, matching how the sprites are
        // lit. In this iso projection screen-up is world (1,1), so equal +x/+y
        // aims the sun at the top of the screen; the large +z keeps it overhead.
        self.material
            .set_uniform("u_sun_dir", vec3(0.38, 0.38, 0.88));
        self.material.set_uniform("u_palette_a", p.palette_a);
        self.material.set_uniform("u_palette_b", p.palette_b);
        self.material.set_uniform("u_palette_c", p.palette_c);
        self.material.set_uniform("u_octaves", p.octaves);
        self.material.set_uniform_array("u_rocks", &rock_buf);
        self.material.set_uniform("u_rock_count", rock_count as i32);
        self.material.set_uniform_array("u_ships", &ship_buf);
        self.material.set_uniform_array("u_ship_dir", &ship_dir_buf);
        self.material
            .set_uniform_array("u_ship_thrust", &ship_thrust_buf);
        self.material.set_uniform("u_ship_count", ship_count as i32);
        self.material.set_uniform_array("u_rockets", &rocket_buf);
        self.material
            .set_uniform_array("u_rocket_dir", &rocket_dir_buf);
        self.material
            .set_uniform("u_rocket_count", rocket_count as i32);

        // Draw a quad that fills the camera's visible area in the render target.
        // A 20-pixel margin prevents edge gaps during camera shake.
        let margin = 20.0;
        gl_use_material(&self.material);
        draw_rectangle(
            view.target.x - vw * 0.5 - margin,
            view.target.y - vh * 0.5 - margin,
            vw + margin * 2.0,
            vh + margin * 2.0,
            WHITE,
        );
        gl_use_default_material();
    }
}
