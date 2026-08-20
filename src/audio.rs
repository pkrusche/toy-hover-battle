use macroquad::audio::{load_sound_from_bytes, play_sound, PlaySoundParams, Sound};

const GUN_FIRE_BYTES: &[u8] = include_bytes!("../assets/sfx/gun_fire.wav");
const ROCKET_LAUNCH_BYTES: &[u8] = include_bytes!("../assets/sfx/rocket_launch.wav");
const SHIELD_HIT_BYTES: &[u8] = include_bytes!("../assets/sfx/shield_hit.wav");
const HULL_HIT_BYTES: &[u8] = include_bytes!("../assets/sfx/hull_hit.wav");
const EXPLOSION_BYTES: &[u8] = include_bytes!("../assets/sfx/explosion.wav");
const VEHICLE_COLLISION_BYTES: &[u8] = include_bytes!("../assets/sfx/vehicle_collision.wav");
const ROCK_IMPACT_BYTES: &[u8] = include_bytes!("../assets/sfx/rock_impact.wav");
const MENU_MOVE_BYTES: &[u8] = include_bytes!("../assets/sfx/menu_move.wav");
const MENU_CONFIRM_BYTES: &[u8] = include_bytes!("../assets/sfx/menu_confirm.wav");
use macroquad::prelude::get_time;

pub enum SfxEvent {
    GunFire,
    RocketLaunch,
    ShieldHit,
    HullHit,
    Explosion,
    VehicleCollision,
    RockImpact,
    MenuMove,
    MenuConfirm,
}

struct Limiter {
    last_played: [f64; 8],
}

impl Limiter {
    fn new() -> Self {
        Self {
            last_played: [f64::NEG_INFINITY; 8],
        }
    }

    fn try_play(&mut self, slot: usize, min_interval: f64) -> bool {
        let now = get_time();
        if now - self.last_played[slot] >= min_interval {
            self.last_played[slot] = now;
            true
        } else {
            false
        }
    }
}

pub struct Sfx {
    gun_fire: Sound,
    rocket_launch: Sound,
    shield_hit: Sound,
    hull_hit: Sound,
    explosion: Sound,
    vehicle_collision: Sound,
    rock_impact: Sound,
    menu_move: Sound,
    menu_confirm: Sound,
    limiter: Limiter,
}

const LIM_GUN: usize = 0;
const LIM_ROCKET_LAUNCH: usize = 1;
const LIM_SHIELD: usize = 3;
const LIM_HULL: usize = 4;
const LIM_VCOLLISION: usize = 5;
const LIM_ROCK: usize = 6;

impl Sfx {
    pub async fn load() -> Self {
        Self {
            gun_fire: load_sound_from_bytes(GUN_FIRE_BYTES)
                .await
                .expect("gun_fire"),
            rocket_launch: load_sound_from_bytes(ROCKET_LAUNCH_BYTES)
                .await
                .expect("rocket_launch"),
            shield_hit: load_sound_from_bytes(SHIELD_HIT_BYTES)
                .await
                .expect("shield_hit"),
            hull_hit: load_sound_from_bytes(HULL_HIT_BYTES)
                .await
                .expect("hull_hit"),
            explosion: load_sound_from_bytes(EXPLOSION_BYTES)
                .await
                .expect("explosion"),
            vehicle_collision: load_sound_from_bytes(VEHICLE_COLLISION_BYTES)
                .await
                .expect("vehicle_collision"),
            rock_impact: load_sound_from_bytes(ROCK_IMPACT_BYTES)
                .await
                .expect("rock_impact"),
            menu_move: load_sound_from_bytes(MENU_MOVE_BYTES)
                .await
                .expect("menu_move"),
            menu_confirm: load_sound_from_bytes(MENU_CONFIRM_BYTES)
                .await
                .expect("menu_confirm"),
            limiter: Limiter::new(),
        }
    }

    pub fn play(&mut self, event: SfxEvent) {
        let p = |volume: f32| PlaySoundParams {
            looped: false,
            volume,
        };
        match event {
            SfxEvent::GunFire => {
                if self.limiter.try_play(LIM_GUN, 0.04) {
                    play_sound(&self.gun_fire, p(0.25));
                }
            }
            SfxEvent::RocketLaunch => {
                if self.limiter.try_play(LIM_ROCKET_LAUNCH, 0.20) {
                    play_sound(&self.rocket_launch, p(0.45));
                }
            }
            SfxEvent::ShieldHit => {
                if self.limiter.try_play(LIM_SHIELD, 0.10) {
                    play_sound(&self.shield_hit, p(0.30));
                }
            }
            SfxEvent::HullHit => {
                if self.limiter.try_play(LIM_HULL, 0.10) {
                    play_sound(&self.hull_hit, p(0.40));
                }
            }
            SfxEvent::Explosion => {
                play_sound(&self.explosion, p(0.60));
            }
            SfxEvent::VehicleCollision => {
                if self.limiter.try_play(LIM_VCOLLISION, 0.20) {
                    play_sound(&self.vehicle_collision, p(0.35));
                }
            }
            SfxEvent::RockImpact => {
                if self.limiter.try_play(LIM_ROCK, 0.22) {
                    play_sound(&self.rock_impact, p(0.30));
                }
            }
            SfxEvent::MenuMove => {
                play_sound(&self.menu_move, p(0.40));
            }
            SfxEvent::MenuConfirm => {
                play_sound(&self.menu_confirm, p(0.50));
            }
        }
    }
}
