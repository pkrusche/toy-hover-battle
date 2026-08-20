use std::collections::HashMap;

use ::rand::{RngExt, SeedableRng};
use fastnoise_lite::{FastNoiseLite, NoiseType};
use macroquad::prelude::*;
use rand_xoshiro::Xoshiro256PlusPlus;

use crate::player::Player;

pub const CHUNK_SIZE: f32 = 32.0; // tiles per chunk side
pub const CELLS_PER_CHUNK: i32 = 8;
pub const CELL: f32 = CHUNK_SIZE / CELLS_PER_CHUNK as f32; // 4.0 tiles

pub struct Rock {
    pub pos: Vec2,
    pub radius: f32,
    pub variant: u8,
    pub hp: f32,
}

impl Rock {
    // Hit points scale with radius so bigger rocks take more hits — at
    // Bullet::DAMAGE (10.0) this is ~2 hits at the smallest radius (0.30) up
    // to ~3 at the largest (0.70), and always more than one shot's damage so a
    // single bullet can never one-shot a rock (a rocket still destroys any
    // rock outright, independent of hp).
    pub const HP_PER_RADIUS: f32 = 35.0;
}

pub struct Chunk {
    pub rocks: Vec<Rock>,
}

pub struct World {
    pub seed: u64,
    pub chunks: HashMap<(i32, i32), Chunk>,
    density: FastNoiseLite,
}

#[inline]
pub fn world_to_chunk(p: Vec2) -> (i32, i32) {
    (
        (p.x / CHUNK_SIZE).floor() as i32,
        (p.y / CHUNK_SIZE).floor() as i32,
    )
}

#[inline]
fn chunk_origin(cx: i32, cy: i32) -> Vec2 {
    vec2(cx as f32 * CHUNK_SIZE, cy as f32 * CHUNK_SIZE)
}

// Splitmix64 finalizer — decorrelates neighboring chunk seeds.
fn chunk_seed(world_seed: u64, cx: i32, cy: i32) -> u64 {
    let x = cx as i64 as u64;
    let y = cy as i64 as u64;
    let mut h = world_seed
        ^ x.wrapping_mul(0x9E3779B97F4A7C15)
        ^ y.wrapping_mul(0xBF58476D1CE4E5B9).rotate_left(17);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    h
}

impl World {
    pub fn new(seed: u64) -> Self {
        let mut density = FastNoiseLite::with_seed(seed as i32);
        density.set_noise_type(Some(NoiseType::OpenSimplex2));
        density.set_frequency(Some(0.04)); // features at ~25-tile scale
        Self {
            seed,
            chunks: HashMap::new(),
            density,
        }
    }

    fn generate_chunk(&self, cx: i32, cy: i32) -> Chunk {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(chunk_seed(self.seed, cx, cy));
        let origin = chunk_origin(cx, cy);
        let mut rocks = Vec::new();

        const THRESHOLD: f32 = -0.1;
        for gy in 0..CELLS_PER_CHUNK {
            for gx in 0..CELLS_PER_CHUNK {
                let wx = origin.x + (gx as f32 + 0.5) * CELL;
                let wy = origin.y + (gy as f32 + 0.5) * CELL;
                let d = self.density.get_noise_2d(wx, wy);
                if d < THRESHOLD {
                    continue;
                }
                let p = ((d - THRESHOLD) / (1.0 - THRESHOLD)).clamp(0.0, 1.0);
                if rng.random::<f32>() > p * 0.8 {
                    continue;
                }
                let margin = 0.25 * CELL;
                let jx: f32 = rng.random_range(margin..CELL - margin);
                let jy: f32 = rng.random_range(margin..CELL - margin);
                let radius = rng.random_range(0.30_f32..0.70);
                rocks.push(Rock {
                    pos: vec2(
                        origin.x + gx as f32 * CELL + jx,
                        origin.y + gy as f32 * CELL + jy,
                    ),
                    radius,
                    variant: rng.random_range(0..16u8),
                    hp: radius * Rock::HP_PER_RADIUS,
                });
            }
        }

        Chunk { rocks }
    }

    pub fn update_loaded(&mut self, players: &[Vec2], view_radius: f32) {
        let mut needed: Vec<(i32, i32)> = Vec::new();
        let r = (view_radius / CHUNK_SIZE).ceil() as i32 + 1;
        for &p in players {
            let (pcx, pcy) = world_to_chunk(p);
            for dy in -r..=r {
                for dx in -r..=r {
                    needed.push((pcx + dx, pcy + dy));
                }
            }
        }
        needed.sort_unstable();
        needed.dedup();

        for &key in &needed {
            if !self.chunks.contains_key(&key) {
                let chunk = self.generate_chunk(key.0, key.1);
                self.chunks.insert(key, chunk);
            }
        }
        let keep: std::collections::HashSet<_> = needed.into_iter().collect();
        self.chunks.retain(|k, _| keep.contains(k));
    }

    pub fn rocks_near(&self, pos: Vec2) -> impl Iterator<Item = &Rock> + '_ {
        let (pcx, pcy) = world_to_chunk(pos);
        let chunks = &self.chunks;
        (-1_i32..=1)
            .flat_map(move |dy| {
                (-1_i32..=1).filter_map(move |dx| chunks.get(&(pcx + dx, pcy + dy)))
            })
            .flat_map(|c| c.rocks.iter())
    }

    /// Like [`rocks_near`], but yields each rock's chunk key and chunk-local
    /// index alongside it — the identity needed to damage or remove one
    /// specific rock later, since position alone isn't stable enough (two
    /// rocks could share a query radius).
    pub fn rocks_near_indexed(
        &self,
        pos: Vec2,
    ) -> impl Iterator<Item = ((i32, i32), usize, &Rock)> + '_ {
        let (pcx, pcy) = world_to_chunk(pos);
        let chunks = &self.chunks;
        (-1_i32..=1)
            .flat_map(move |dy| {
                (-1_i32..=1).filter_map(move |dx| {
                    let key = (pcx + dx, pcy + dy);
                    chunks.get(&key).map(|c| (key, c))
                })
            })
            .flat_map(|(key, c)| c.rocks.iter().enumerate().map(move |(i, r)| (key, i, r)))
    }

    /// Apply weapon damage to the rock at `(chunk_key, index)`. Returns the
    /// rock's position if this brought it to zero hp (it is removed from the
    /// chunk immediately, so the caller can spawn destruction VFX there), or
    /// `None` if it survived or no longer exists.
    pub fn damage_rock(
        &mut self,
        chunk_key: (i32, i32),
        index: usize,
        amount: f32,
    ) -> Option<Vec2> {
        let chunk = self.chunks.get_mut(&chunk_key)?;
        let rock = chunk.rocks.get_mut(index)?;
        rock.hp -= amount;
        if rock.hp <= 0.0 {
            let pos = rock.pos;
            chunk.rocks.remove(index);
            Some(pos)
        } else {
            None
        }
    }

    /// Destroy the rock at `(chunk_key, index)` outright, regardless of
    /// remaining hp (a direct rocket hit). Returns its position for VFX, or
    /// `None` if it no longer exists.
    pub fn destroy_rock(&mut self, chunk_key: (i32, i32), index: usize) -> Option<Vec2> {
        let chunk = self.chunks.get_mut(&chunk_key)?;
        if index >= chunk.rocks.len() {
            return None;
        }
        Some(chunk.rocks.remove(index).pos)
    }
}

// Returns true if any rock impact exceeded the damage-threshold speed.
pub fn resolve_vehicle_rocks(p: &mut Player, world: &World) -> bool {
    let mut had_impact = false;
    for rock in world.rocks_near(p.pos) {
        let delta = p.pos - rock.pos;
        let min_dist = p.radius + rock.radius;
        let dist2 = delta.length_squared();
        if dist2 < min_dist * min_dist && dist2 > 1e-4 {
            let dist = dist2.sqrt();
            p.pos += delta * ((min_dist - dist) / dist);
            let n = delta / dist;
            let into = p.vel.dot(n);
            if into < 0.0 {
                p.vel -= n * into;
                let impact = -into;
                if impact > Player::COLLISION_MIN_SPEED {
                    had_impact = true;
                    if p.shield <= 0.0 {
                        p.apply_damage(
                            (impact - Player::COLLISION_MIN_SPEED) * Player::COLLISION_DAMAGE_SCALE,
                        );
                    }
                }
            }
        }
    }
    had_impact
}
