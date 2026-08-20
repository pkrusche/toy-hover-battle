use macroquad::prelude::*;

use crate::{audio::SfxEvent, iso::world_to_screen, player::Player, world::World};

pub struct Bullet {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: f32,
    pub owner: u8,
    pub damage: f32,
}

pub struct Rocket {
    pub pos: Vec2,
    pub vel: Vec2,
    pub angle: f32,
    pub life: f32,
    pub owner: u8,
    pub damage: f32,
}

/// A rocket that just exploded (on impact or at end of life). The caller turns
/// each one into an `Explosion` (visual blast + area damage). The blast's splash
/// catches every nearby player, including one struck head-on — the reduced
/// direct damage already accounts for that overlap.
pub struct Detonation {
    pub pos: Vec2,
}

impl Bullet {
    pub const SPEED: f32 = 22.0; // tiles/s
    pub const LIFE: f32 = 0.5; // seconds
    pub const RADIUS: f32 = 0.12; // tiles
    pub const DAMAGE: f32 = 10.0;

    pub fn spawn_from(p: &Player, owner: u8) -> Self {
        // Fires along the turret, not the hull — the turret tracks the aim
        // point within its traverse cone (see `Player::turret_angle`), which
        // is what lets a ship thrust across the enemy's face and keep firing.
        let heading = vec2(p.turret_angle.cos(), p.turret_angle.sin());
        Self {
            pos: p.pos + heading * (p.radius + 0.15),
            // Fixed muzzle velocity: bullets always travel at exactly SPEED along
            // the turret, independent of the firing ship's own velocity.
            vel: heading * Self::SPEED,
            life: Self::LIFE,
            owner,
            damage: Self::DAMAGE,
        }
    }

    fn update(&mut self, dt: f32) {
        self.pos += self.vel * dt;
        self.life -= dt;
    }
}

impl Rocket {
    pub const INITIAL_SPEED: f32 = 4.0;
    pub const MAX_SPEED: f32 = 20.0;
    pub const ACCEL: f32 = 4.8;
    pub const STEER_RATE: f32 = 3.2;
    pub const LIFE: f32 = 8.0;
    pub const RADIUS: f32 = 0.2;
    // Direct hits now also catch the target in the blast splash (~40 at point-blank),
    // so the direct component is lowered to keep the two-hit-kill balance:
    // direct (50) + splash (~40) ≈ 90 per hit vs. a full-health pool of 175.
    pub const DAMAGE: f32 = 50.0;

    pub fn spawn_from(p: &Player, owner: u8) -> Self {
        let heading = vec2(p.angle.cos(), p.angle.sin());
        // Inherit the ship's motion, but strip any *reverse* (backward) component
        // first. INITIAL_SPEED (4 tiles/s) is well under the ~7 tiles/s reverse
        // top speed, so without this a rocket fired while retreating would launch
        // backward and curl around awkwardly instead of leaving the nose forward.
        // Sideways/forward carry is kept for a natural launch.
        let reverse = p.vel.dot(heading).min(0.0);
        let carry = p.vel - heading * reverse;
        Self {
            pos: p.pos + heading * (p.radius + 0.28),
            vel: carry + heading * Self::INITIAL_SPEED,
            angle: p.angle,
            life: Self::LIFE,
            owner,
            damage: Self::DAMAGE,
        }
    }

    fn update(&mut self, target_pos: Vec2, dt: f32) {
        let current_dir = self.vel.normalize_or_zero();
        let desired_dir = (target_pos - self.pos).normalize_or_zero();
        let steer = (Self::STEER_RATE * dt).clamp(0.0, 1.0);
        let dir = current_dir.lerp(desired_dir, steer).normalize_or_zero();

        let speed = (self.vel.length() + Self::ACCEL * dt).min(Self::MAX_SPEED);
        let dir = if dir.length_squared() > 1e-6 {
            dir
        } else {
            desired_dir
        };
        self.vel = dir * speed;
        self.angle = dir.y.atan2(dir.x);
        self.pos += self.vel * dt;
        self.life -= dt;
    }
}

pub fn update_bullets(
    bullets: &mut Vec<Bullet>,
    players: &mut [Player; 2],
    world: &mut World,
    dt: f32,
    events: &mut Vec<SfxEvent>,
    rock_destructions: &mut Vec<Vec2>,
) {
    bullets.retain_mut(|b| {
        b.update(dt);
        if b.life <= 0.0 {
            return false;
        }
        let hit_rock = world
            .rocks_near_indexed(b.pos)
            .find(|(_, _, rock)| {
                b.pos.distance_squared(rock.pos) < (rock.radius + Bullet::RADIUS).powi(2)
            })
            .map(|(key, idx, _)| (key, idx));
        if let Some((key, idx)) = hit_rock {
            if let Some(pos) = world.damage_rock(key, idx, b.damage) {
                rock_destructions.push(pos);
            }
            return false;
        }
        for (i, p) in players.iter_mut().enumerate() {
            if i as u8 == b.owner || p.is_dead() {
                continue;
            }
            if b.pos.distance_squared(p.pos) < (p.radius + Bullet::RADIUS).powi(2) {
                events.push(if p.shield > 0.0 {
                    SfxEvent::ShieldHit
                } else {
                    SfxEvent::HullHit
                });
                p.apply_damage(b.damage);
                return false;
            }
        }
        true
    });
}

pub fn update_rockets(
    rockets: &mut Vec<Rocket>,
    players: &mut [Player; 2],
    world: &mut World,
    dt: f32,
    events: &mut Vec<SfxEvent>,
    detonations: &mut Vec<Detonation>,
    rock_destructions: &mut Vec<Vec2>,
) {
    rockets.retain_mut(|r| {
        let target_idx = 1 - r.owner as usize;
        if players[target_idx].is_dead() {
            // No one left to home; go off where it is rather than vanishing.
            detonations.push(Detonation { pos: r.pos });
            return false;
        }
        let target_pos = players[target_idx].pos;
        r.update(target_pos, dt);
        if r.life <= 0.0 {
            detonations.push(Detonation { pos: r.pos });
            return false;
        }
        let hit_rock = world
            .rocks_near_indexed(r.pos)
            .find(|(_, _, rock)| {
                r.pos.distance_squared(rock.pos) < (rock.radius + Rocket::RADIUS).powi(2)
            })
            .map(|(key, idx, _)| (key, idx));
        if let Some((key, idx)) = hit_rock {
            // A direct rocket hit destroys the rock outright, independent of
            // its remaining hp.
            if let Some(pos) = world.destroy_rock(key, idx) {
                rock_destructions.push(pos);
            }
            detonations.push(Detonation { pos: r.pos });
            return false;
        }
        for (i, p) in players.iter_mut().enumerate() {
            if i as u8 == r.owner || p.is_dead() {
                continue;
            }
            if r.pos.distance_squared(p.pos) < (p.radius + Rocket::RADIUS).powi(2) {
                events.push(if p.shield > 0.0 {
                    SfxEvent::ShieldHit
                } else {
                    SfxEvent::HullHit
                });
                p.apply_damage(r.damage);
                detonations.push(Detonation { pos: r.pos });
                return false;
            }
        }
        true
    });
}

pub fn draw_bullet(b: &Bullet) {
    let s = world_to_screen(b.pos);
    draw_circle(s.x, s.y, 16.0, Color::new(1.0, 0.55, 0.05, 0.065));
    draw_circle(s.x, s.y, 10.0, Color::new(1.0, 0.7, 0.08, 0.13));
    draw_circle(s.x, s.y, 4.0, YELLOW);
    draw_circle(s.x, s.y, 2.0, WHITE);
}
