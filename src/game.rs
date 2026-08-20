use std::collections::VecDeque;
use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use macroquad::prelude::*;

use crate::{
    ai::{AiAgent, AiDebug, ControllerKind, PlayerView},
    assets::{ExplosionSprites, RockSprites, RocketSprites, ShipSprites},
    audio::SfxEvent,
    bullet::{self, Bullet, Detonation, Rocket},
    iso::{world_to_screen, y_sort_key, TH, TW},
    pads::Pads,
    player::{Player, PlayerInput},
    world::{self, World},
};

// Split-screen viewport size (one render target per player).
pub const VW: u32 = 480;
pub const VH: u32 = 540;
// Single-screen viewport: same height and pixel scale, twice as wide, so it
// fills the window when only one viewpoint is drawn.
pub const VW_FULL: u32 = VW * 2;

/// How the frame is composited. With exactly one human in the match there is
/// only one viewpoint worth showing, so it gets the whole window; two humans —
/// or an AI-vs-AI spectator match, where both sides are worth watching — keep
/// the side-by-side split.
#[derive(Clone, Copy, PartialEq)]
pub enum Layout {
    Single { viewer: usize },
    Split,
}

impl Layout {
    /// Viewport size in render-target pixels for this layout.
    pub fn view_size(self) -> Vec2 {
        match self {
            Layout::Single { .. } => vec2(VW_FULL as f32, VH as f32),
            Layout::Split => vec2(VW as f32, VH as f32),
        }
    }
}

/// Match pacing, selected on the setup screen alongside each player's
/// controller kind. Scales the simulation's `dt` uniformly — physics,
/// cooldowns, timers, AI reaction — so a slower match is more forgiving end
/// to end rather than just making the ships themselves sluggish.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameSpeed {
    Slow,
    Normal,
    Fast,
}

impl GameSpeed {
    pub fn label(self) -> &'static str {
        match self {
            GameSpeed::Slow => "Slow",
            GameSpeed::Normal => "Normal",
            GameSpeed::Fast => "Fast",
        }
    }

    /// Multiplier applied to `dt` at the top of `GameState::update`.
    fn scale(self) -> f32 {
        match self {
            GameSpeed::Slow => 0.75,
            GameSpeed::Normal => 1.0,
            GameSpeed::Fast => 1.3,
        }
    }

    /// Cycle Slow → Normal → Fast → Slow, for the setup screen.
    pub fn next(self) -> Self {
        match self {
            GameSpeed::Slow => GameSpeed::Normal,
            GameSpeed::Normal => GameSpeed::Fast,
            GameSpeed::Fast => GameSpeed::Slow,
        }
    }

    /// Reverse of [`next`], for the setup screen's left arrow.
    pub fn prev(self) -> Self {
        match self {
            GameSpeed::Slow => GameSpeed::Fast,
            GameSpeed::Normal => GameSpeed::Slow,
            GameSpeed::Fast => GameSpeed::Normal,
        }
    }
}

const PLAYER_COLORS: [Color; 2] = [
    Color::new(0.2, 0.6, 1.0, 1.0),
    Color::new(1.0, 0.35, 0.2, 1.0),
];

const VIEW_RADIUS: f32 = 24.0;
// Screen-space padding (px) added to each side of the viewport when culling
// sprites — covers the largest sprite's half-extent plus camera-shake offset.
const CULL_MARGIN: f32 = 120.0;
const FIRE_COOLDOWN: f32 = 0.18;
const MISSILE_COOLDOWN: f32 = 1.0;
const DEATH_VIEW_FADE_TIME: f32 = 0.6;

// ── Particles ────────────────────────────────────────────────────────────────

struct Particle {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    max_life: f32,
    radius: f32,
    color: Color,
}

impl Particle {
    // Returns false when expired.
    fn update(&mut self, dt: f32) -> bool {
        self.pos += self.vel * dt;
        self.vel *= (1.0 - 4.0 * dt).max(0.0); // drag — slows quickly
        self.life -= dt;
        self.life > 0.0
    }

    fn draw(&self) {
        let alpha = (self.life / self.max_life).max(0.0);
        let s = world_to_screen(self.pos);
        let c = self.color;
        draw_circle(s.x, s.y, self.radius, Color::new(c.r, c.g, c.b, alpha));
    }
}

// ── Smoke particles ───────────────────────────────────────────────────────────

struct SmokeParticle {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    max_life: f32,
    radius: f32, // screen pixels
    shade: f32,  // 0 = black, 1 = white
}

impl SmokeParticle {
    fn draw(&self) {
        // Fade out over the full lifetime; smoke is semi-transparent throughout.
        let alpha = (self.life / self.max_life) * 0.45;
        let s = world_to_screen(self.pos);
        let g = self.shade;
        draw_circle(s.x, s.y, self.radius, Color::new(g, g, g, alpha));
    }
}

// ── Explosion ────────────────────────────────────────────────────────────────

struct Explosion {
    pos: Vec2,
    life: f32,
    max_life: f32,
    particles: Vec<Particle>,
    // Whether each player has already been caught by the expanding shockwave
    // front — the blast hits a given player at most once per explosion.
    shock_hit: [bool; 2],
    // Size multiplier on the ring/particle geometry and the sprite burst —
    // 1.0 for a full ship-kill/rocket blast, smaller for a rock destruction.
    scale: f32,
    // Whether the expanding shockwave applies knockback/damage to players.
    // Off for rock destructions — those are a visual beat only.
    damages_players: bool,
}

impl Explosion {
    const N_PARTICLES: usize = 16;
    // Expanding ring geometry, in world tiles. The drawn ring and the shockwave
    // front share these so the blast lands exactly where the ring is rendered:
    //   radius(t) = (1 - t) * RING_GROWTH + RING_BASE,  t = life/max_life (1→0)
    const RING_BASE: f32 = 0.3;
    const RING_GROWTH: f32 = 2.8;
    const RING_MAX: f32 = Self::RING_BASE + Self::RING_GROWTH; // 3.1 tiles
                                                               // Blast effect at the centre, falling off linearly to zero at RING_MAX.
    const KNOCKBACK_MAX: f32 = 9.0; // tiles/s outward impulse
    const DAMAGE_MAX: f32 = 40.0; // hull/shield HP
                                  // Rock destructions reuse the same effect at a fraction of the size, with
                                  // no splash damage to players (see `damages_players`).
    const ROCK_SCALE: f32 = 0.45;

    fn new(pos: Vec2) -> Self {
        Self::with_scale(pos, 1.0, true)
    }

    fn new_rock_destruction(pos: Vec2) -> Self {
        Self::with_scale(pos, Self::ROCK_SCALE, false)
    }

    fn with_scale(pos: Vec2, scale: f32, damages_players: bool) -> Self {
        // Seed the burst angle from wall-clock time so repeated deaths look different.
        let seed_angle = (get_time() as f32 * 13.71).fract() * TAU;
        let mut particles = Vec::with_capacity(Self::N_PARTICLES);

        for i in 0..Self::N_PARTICLES {
            let frac = i as f32 / Self::N_PARTICLES as f32;
            let angle = seed_angle + frac * TAU;
            // Speed and lifetime vary by thirds so we get three distinct "rings".
            let tier = i % 3;
            let speed = (3.0 + tier as f32 * 1.4) * scale; // 3.0 / 4.4 / 5.8 tiles/s
            let life = 0.38 + tier as f32 * 0.12; // 0.38 / 0.50 / 0.62 s
            let radius = (3.5 - tier as f32 * 0.7) * scale; // 3.5 / 2.8 / 2.1 px
            let color = match tier {
                0 => Color::new(1.0, 0.88, 0.25, 1.0),  // bright yellow
                1 => Color::new(1.0, 0.48, 0.10, 1.0),  // orange
                _ => Color::new(0.85, 0.18, 0.08, 1.0), // deep red
            };
            particles.push(Particle {
                pos,
                vel: vec2(angle.cos(), angle.sin()) * speed,
                life,
                max_life: life,
                radius,
                color,
            });
        }

        Self {
            pos,
            life: 0.75,
            max_life: 0.75,
            particles,
            shock_hit: [false; 2],
            scale,
            damages_players,
        }
    }

    // Current world-space radius of the expanding ring front (tiles).
    fn ring_radius(&self) -> f32 {
        let t = (self.life / self.max_life).max(0.0); // 1 → 0
        ((1.0 - t) * Self::RING_GROWTH + Self::RING_BASE) * self.scale
    }

    // Returns false when both ring and all particles have expired.
    fn update(&mut self, dt: f32) -> bool {
        self.life -= dt;
        self.particles.retain_mut(|p| p.update(dt));
        self.life > 0.0 || !self.particles.is_empty()
    }

    // The blast renders in three primitive groups — filled circles/ellipses, the
    // ring line, then the sprite-sheet burst. They're split into separate methods
    // so the caller can draw all explosions' fills, then all rings, then all
    // sprites, keeping each kind in one batch (see draw_world's effects pass).
    // Intra-blast layering is preserved: a given explosion's fills sit under its
    // ring, which sits under its sprite.

    // Broad environmental flash at ignition. Concentric low-alpha layers soften
    // the edge; the area contracts slightly while its intensity fades rapidly.
    fn draw_glow(&self) {
        if self.life <= 0.0 {
            return;
        }
        let t = (self.life / self.max_life).clamp(0.0, 1.0); // 1 → 0
        let fade = t * t;
        let radius = (78.0 + 32.0 * t) * self.scale;
        let sc = world_to_screen(self.pos);
        draw_circle(
            sc.x,
            sc.y,
            radius,
            Color::new(1.0, 0.28, 0.03, fade * 0.045),
        );
        draw_circle(
            sc.x,
            sc.y,
            radius * 0.68,
            Color::new(1.0, 0.48, 0.05, fade * 0.09),
        );
        draw_circle(
            sc.x,
            sc.y,
            radius * 0.38,
            Color::new(1.0, 0.75, 0.12, fade * 0.16),
        );
    }

    // Filled spark particles plus, early on, the bright core (default white
    // texture — batches with smoke and muzzle flashes).
    fn draw_fills(&self) {
        for p in &self.particles {
            p.draw();
        }
        if self.life > 0.0 {
            let t = (self.life / self.max_life).max(0.0); // 1 → 0
            if t > 0.65 {
                let sc = world_to_screen(self.pos);
                let k = FRAC_1_SQRT_2;
                let core_t = (t - 0.65) / 0.35;
                let cr = (1.0 - t) * TW * k * 0.7 * self.scale;
                let cry = (1.0 - t) * TH * k * 0.7 * self.scale;
                draw_ellipse(
                    sc.x,
                    sc.y,
                    cr.max(1.0),
                    cry.max(0.5),
                    0.0,
                    Color::new(1.0, 0.85, 0.2, core_t * 0.7),
                );
            }
        }
    }

    // Expanding outer ring (radius shared with the shockwave front) — a line
    // primitive, so it batches apart from the filled spark particles.
    fn draw_ring(&self) {
        if self.life <= 0.0 {
            return;
        }
        let t = (self.life / self.max_life).max(0.0); // 1 → 0
        let sc = world_to_screen(self.pos);
        let k = FRAC_1_SQRT_2;
        let r = self.ring_radius();
        let rx = r * TW * k;
        let ry = r * TH * k;
        let g = t * 0.55 + 0.15;
        draw_ellipse_lines(
            sc.x,
            sc.y,
            rx,
            ry,
            0.0,
            3.0,
            Color::new(1.0, g, 0.0, t * 0.85),
        );
    }

    // Sprite-sheet burst on top of the procedural effect. Progress runs 0 → 1
    // over the ring's lifetime, advancing through all 60 frames. Shares the
    // explosion atlas across every blast.
    fn draw_sprite(&self, expl: &ExplosionSprites) {
        let progress = (1.0 - self.life / self.max_life).clamp(0.0, 1.0);
        expl.draw(self.pos, progress, self.scale);
    }
}

// ── GameState ─────────────────────────────────────────────────────────────────

pub struct GameState {
    pub players: [Player; 2],
    /// Who controls each slot. Retained across respawns.
    controllers: [ControllerKind; 2],
    /// Match pacing selected on the setup screen — see [`GameSpeed`].
    speed: GameSpeed,
    /// One persistent agent per AI-controlled slot; `None` for human slots.
    /// Kept across respawns so AI state survives death.
    agents: [Option<AiAgent>; 2],
    /// F3 debug overlay toggle for AI-controlled viewports.
    show_ai_debug: bool,
    pub bullets: Vec<Bullet>,
    pub rockets: Vec<Rocket>,
    pub world: World,
    explosions: Vec<Explosion>,
    particles: Vec<Particle>,
    smoke: Vec<SmokeParticle>,
    smoke_timers: [f32; 2],
    smoke_seq: u32,
    pub scores: [u32; 2],
    score_flash_timer: f32,
    pub draw_ms: f32,
    frame_times: VecDeque<f32>,
    fps_sum: f32,
    fps_avg: f32,
    pub sfx_events: Vec<SfxEvent>,
}

impl GameState {
    pub fn new(controllers: [ControllerKind; 2], speed: GameSpeed) -> Self {
        // One persistent agent per AI slot, seeded deterministically per slot.
        let agents = [
            Self::make_agent(0, controllers[0]),
            Self::make_agent(1, controllers[1]),
        ];
        Self {
            players: [Player::new(vec2(-3.0, 0.0)), Player::new(vec2(3.0, 0.0))],
            controllers,
            speed,
            agents,
            show_ai_debug: false,
            bullets: Vec::new(),
            rockets: Vec::new(),
            world: World::new(12345),
            explosions: Vec::new(),
            particles: Vec::new(),
            smoke: Vec::new(),
            smoke_timers: [0.0; 2],
            smoke_seq: 0,
            scores: [0; 2],
            score_flash_timer: 0.0,
            draw_ms: 0.0,
            frame_times: VecDeque::new(),
            fps_sum: 0.0,
            fps_avg: 0.0,
            sfx_events: Vec::new(),
        }
    }

    fn make_agent(idx: usize, kind: ControllerKind) -> Option<AiAgent> {
        match kind {
            ControllerKind::Ai(diff) => {
                // Deterministic base seed per slot; varies by difficulty.
                let seed = 0xA1_5EEDu64
                    .wrapping_mul(idx as u64 + 1)
                    .wrapping_add(diff as u64 * 0x9E37);
                Some(AiAgent::new(idx, diff, seed))
            }
            ControllerKind::Human => None,
        }
    }

    /// Single-screen when exactly one slot is human, split otherwise.
    pub fn layout(&self) -> Layout {
        match (self.controllers[0].is_ai(), self.controllers[1].is_ai()) {
            (false, true) => Layout::Single { viewer: 0 },
            (true, false) => Layout::Single { viewer: 1 },
            _ => Layout::Split,
        }
    }

    pub fn drain_sfx_events(&mut self) -> impl Iterator<Item = SfxEvent> + '_ {
        self.sfx_events.drain(..)
    }

    pub fn update(&mut self, dt: f32, pads: &mut Pads) {
        // Rolling 1-second FPS average.
        self.frame_times.push_back(dt);
        self.fps_sum += dt;
        while self.fps_sum > 1.0 && self.frame_times.len() > 1 {
            self.fps_sum -= self.frame_times.pop_front().unwrap();
        }
        self.fps_avg = self.frame_times.len() as f32 / self.fps_sum;

        // From here on `dt` drives the simulation and carries the match's
        // speed setting — physics, cooldowns, timers, and AI reaction all
        // read this scaled value, so play genuinely speeds up or slows down
        // rather than just moving the ships faster.
        let dt = dt * self.speed.scale();

        if is_key_pressed(KeyCode::F3) {
            self.show_ai_debug = !self.show_ai_debug;
        }

        // Load world chunks *before* generating input so AI obstacle queries are
        // valid on the very first frame (and after any teleport/respawn).
        let positions = [self.players[0].pos, self.players[1].pos];
        self.world.update_loaded(&positions, VIEW_RADIUS);

        // Snapshot both players before ticking any agent, so AI-vs-AI decisions
        // are independent of which agent evaluates first.
        let views = [
            PlayerView::of(&self.players[0]),
            PlayerView::of(&self.players[1]),
        ];

        let mut inputs: [PlayerInput; 2] = [PlayerInput::default(), PlayerInput::default()];
        for i in 0..2 {
            match self.controllers[i] {
                ControllerKind::Human => {
                    let mut inp = if i == 0 {
                        read_input_p1()
                    } else {
                        read_input_p2()
                    };
                    pads.merge_into(i, &mut inp);
                    // No separate aim control: the turret auto-tracks the
                    // opponent within its traverse cone (see turret_angle).
                    inp.turret_target = views[1 - i].pos;
                    inputs[i] = inp;
                }
                ControllerKind::Ai(_) => {
                    if let Some(agent) = self.agents[i].as_mut() {
                        inputs[i] = agent.tick(views[i], views[1 - i], &self.world, dt);
                    }
                }
            }
        }
        let [i0, i1] = inputs;

        for i in 0..2 {
            self.players[i].tick_timers(dt);
        }

        // Physics — skip dead players.
        for (i, input) in [&i0, &i1].iter().enumerate() {
            if !self.players[i].is_dead() {
                self.players[i].update(input, dt);
            }
        }

        // Bullet spawning — skip dead players. The heat gate applies uniformly
        // to human and AI controllers alike: both drive `input.fire` through
        // this same check, there is no separate AI fire path.
        for (idx, input) in [&i0, &i1].iter().enumerate() {
            if !self.players[idx].is_dead()
                && input.fire
                && self.players[idx].try_fire(FIRE_COOLDOWN)
            {
                let b = Bullet::spawn_from(&self.players[idx], idx as u8);
                self.particles.push(Particle {
                    pos: b.pos,
                    vel: Vec2::ZERO,
                    life: 0.07,
                    max_life: 0.07,
                    radius: 6.0,
                    color: Color::new(1.0, 0.95, 0.55, 1.0),
                });
                self.sfx_events.push(SfxEvent::GunFire);
                self.bullets.push(b);
            }
        }

        // Rocket spawning — keypress only, limited to 2 per life with 1 s interval.
        for (idx, input) in [&i0, &i1].iter().enumerate() {
            if !self.players[idx].is_dead()
                && input.fire_rocket
                && self.players[idx].missile_count > 0
                && self.players[idx].missile_cooldown <= 0.0
            {
                self.players[idx].missile_count -= 1;
                self.players[idx].missile_cooldown = MISSILE_COOLDOWN;
                let r = Rocket::spawn_from(&self.players[idx], idx as u8);
                self.particles.push(Particle {
                    pos: r.pos,
                    vel: Vec2::ZERO,
                    life: 0.12,
                    max_life: 0.12,
                    radius: 8.0,
                    color: Color::new(1.0, 0.55, 0.18, 1.0),
                });
                self.sfx_events.push(SfxEvent::RocketLaunch);
                self.rockets.push(r);
            }
        }

        for i in 0..2 {
            if !self.players[i].is_dead()
                && world::resolve_vehicle_rocks(&mut self.players[i], &self.world)
            {
                self.sfx_events.push(SfxEvent::RockImpact);
            }
        }

        // Vehicle-vehicle collision only while both alive.
        if !self.players[0].is_dead()
            && !self.players[1].is_dead()
            && resolve_players(&mut self.players)
        {
            self.sfx_events.push(SfxEvent::VehicleCollision);
        }

        let mut rock_destructions: Vec<Vec2> = Vec::new();
        bullet::update_bullets(
            &mut self.bullets,
            &mut self.players,
            &mut self.world,
            dt,
            &mut self.sfx_events,
            &mut rock_destructions,
        );
        let mut detonations: Vec<Detonation> = Vec::new();
        bullet::update_rockets(
            &mut self.rockets,
            &mut self.players,
            &mut self.world,
            dt,
            &mut self.sfx_events,
            &mut detonations,
            &mut rock_destructions,
        );

        // Each rocket detonation spawns an explosion. Its expanding shockwave (see
        // apply_explosion_shockwaves) deals area damage + knockback to every nearby
        // living player, including one the rocket struck head-on (the reduced direct
        // damage accounts for that overlap).
        for det in detonations {
            self.explosions.push(Explosion::new(det.pos));
            self.sfx_events.push(SfxEvent::Explosion);
        }

        // Destroyed rocks get a small, damage-less explosion — no shockwave
        // applied to players (see `apply_explosion_shockwaves`), just the
        // visual beat. The background shader naturally stops perturbing the
        // ground there next frame since it rebuilds `u_rocks` from
        // `world.rocks_near` every frame.
        for pos in rock_destructions {
            self.explosions.push(Explosion::new_rock_destruction(pos));
            self.sfx_events.push(SfxEvent::RockImpact);
        }

        // Detect fresh kills (hull just hit 0).
        for i in 0..2 {
            if self.players[i].hull <= 0.0 && !self.players[i].is_dead() {
                self.players[i].respawn_timer = Player::RESPAWN_DELAY;
                self.players[i].camera_shake = 0.0;
                self.explosions.push(Explosion::new(self.players[i].pos));
                let dead = i as u8;
                self.bullets.retain(|b| b.owner != dead);
                // Rockets outlive their owner: they keep homing the killer as a
                // parting shot. The hit loop in update_rockets skips the owner,
                // so a respawned owner won't be struck by their own missile.
                self.scores[1 - i] += 1;
                self.score_flash_timer = 2.5;
                self.sfx_events.push(SfxEvent::Explosion);
            }
        }

        // Tick respawn timers.
        for i in 0..2 {
            if self.players[i].is_dead() {
                self.players[i].respawn_timer -= dt;
                if self.players[i].respawn_timer <= 0.0 {
                    self.players[i].respawn_timer = 0.0;
                    self.do_respawn(i);
                }
            }
        }

        self.explosions.retain_mut(|e| e.update(dt));
        self.apply_explosion_shockwaves();
        self.particles.retain_mut(|p| p.update(dt));
        self.score_flash_timer = (self.score_flash_timer - dt).max(0.0);

        // Smoke emission from damaged players.
        for i in 0..2 {
            if self.players[i].is_dead() {
                continue;
            }
            let hull_frac = self.players[i].hull / Player::MAX_HULL;
            if hull_frac >= 0.6 {
                continue;
            }

            self.smoke_timers[i] -= dt;
            if self.smoke_timers[i] <= 0.0 {
                self.smoke_timers[i] = if hull_frac < 0.3 { 0.07 } else { 0.14 };
                let count: usize = if hull_frac < 0.3 { 2 } else { 1 };
                let pos = self.players[i].pos;
                let base_t = get_time() as f32;
                for _ in 0..count {
                    let seq = self.smoke_seq;
                    self.smoke_seq += 1;
                    let angle = (base_t * 7.31 + seq as f32 * 2.399).fract() * TAU;
                    let speed = 0.4 + (base_t * 13.7 + seq as f32 * 1.618).fract() * 1.1;
                    let radius = 3.5 + (base_t * 5.13 + seq as f32 * 3.147).fract() * 6.5;
                    let shade = 0.15 + (base_t * 9.99 + seq as f32 * 1.234).fract() * 0.35;
                    let life = 0.8 + (base_t * 3.77 + seq as f32 * 4.321).fract() * 1.2;
                    self.smoke.push(SmokeParticle {
                        pos,
                        vel: vec2(angle.cos(), angle.sin()) * speed,
                        life,
                        max_life: life,
                        radius,
                        shade,
                    });
                }
            }
        }
        self.smoke.retain_mut(|s| {
            s.pos += s.vel * dt;
            s.life -= dt;
            s.life > 0.0
        });
    }

    // Shockwave: as each explosion's ring expands, the moment its front sweeps
    // over a living player they get shoved outward and take damage — once per
    // explosion. Strength falls off with distance from the blast centre, so a
    // player caught point-blank is hit hard and one grazed at the ring's edge
    // barely feels it.
    fn apply_explosion_shockwaves(&mut self) {
        for e in &mut self.explosions {
            // Only while the ring is still drawn/expanding.
            if e.life <= 0.0 || !e.damages_players {
                continue;
            }
            let radius = e.ring_radius();
            for i in 0..2 {
                if e.shock_hit[i] || self.players[i].is_dead() {
                    continue;
                }
                let delta = self.players[i].pos - e.pos;
                let d = delta.length();
                // The front has just reached this player's hull.
                if d > radius + self.players[i].radius {
                    continue;
                }
                e.shock_hit[i] = true;

                // 1 at the centre → 0 at the ring's maximum reach.
                let strength = (1.0 - d / Explosion::RING_MAX).clamp(0.0, 1.0);
                let dir = if d > 1e-3 {
                    delta / d
                } else {
                    // Degenerate: blast centred on the player — pick a fixed axis.
                    vec2(1.0, 0.0)
                };
                self.players[i].vel += dir * (strength * Explosion::KNOCKBACK_MAX);
                self.players[i].apply_damage(strength * Explosion::DAMAGE_MAX);
            }
        }
    }

    fn do_respawn(&mut self, idx: usize) {
        // Place spawner ~12 tiles away from the other player in a random direction.
        let other_pos = self.players[1 - idx].pos;
        let angle = (get_time() as f32 * 17.37 + idx as f32 * 3.17).fract() * TAU;
        let spawn = other_pos + vec2(angle.cos(), angle.sin()) * 12.0;

        self.players[idx].pos = spawn;
        self.players[idx].vel = Vec2::ZERO;
        self.players[idx].hull = Player::MAX_HULL;
        self.players[idx].shield = Player::MAX_SHIELD;
        self.players[idx].shield_recharge_timer = Player::SHIELD_RECHARGE_DELAY;
        self.players[idx].missile_count = Player::MAX_MISSILES;
        self.players[idx].missile_cooldown = 0.0;

        // Push clear of rocks.
        for _ in 0..8 {
            world::resolve_vehicle_rocks(&mut self.players[idx], &self.world);
        }
    }

    pub fn death_view_fade(&self, idx: usize) -> f32 {
        let p = &self.players[idx];
        if !p.is_dead() {
            return 0.0;
        }

        ((Player::RESPAWN_DELAY - p.respawn_timer) / DEATH_VIEW_FADE_TIME).clamp(0.0, 1.0)
    }

    pub fn draw_world(
        &self,
        viewer: usize,
        view_size: Vec2,
        ships: &ShipSprites,
        rocks: &RockSprites,
        expl: &ExplosionSprites,
        rocket_sprites: &RocketSprites,
    ) {
        // Per-viewport cull: only items whose screen position falls inside this
        // viewer's viewport (plus a margin for sprite extent + camera shake) are
        // worth drawing. Without this, every loaded chunk — including the rocks
        // around the *other* player, off-screen — is submitted for both viewports.
        let cull_center = world_to_screen(self.players[viewer].pos);
        let cull_half = view_size * 0.5 + vec2(CULL_MARGIN, CULL_MARGIN);
        let visible = |wp: Vec2| {
            let s = world_to_screen(wp);
            (s.x - cull_center.x).abs() <= cull_half.x && (s.y - cull_center.y).abs() <= cull_half.y
        };

        // Shadow pre-pass — drawn on the ground plane before any sprites. Each
        // object kind shares one shadow atlas, so its shadows batch into a single
        // GPU draw call. Interleaving them with the y-sorted sprite pass below
        // would force a texture switch per item.
        for chunk in self.world.chunks.values() {
            for rock in &chunk.rocks {
                if visible(rock.pos) {
                    rocks.draw_shadow(rock);
                }
            }
        }
        for i in 0..2 {
            if !self.players[i].is_dead() {
                ships.draw_shadow(self.players[i].angle, world_to_screen(self.players[i].pos));
            }
        }
        for rocket in &self.rockets {
            if visible(rocket.pos) {
                rocket_sprites.draw_shadow(rocket.angle, rocket.pos);
            }
        }

        // Depth-sorted pass — only the sprites that must occlude correctly against
        // each other: rocks, rockets, and ships. Everything else (smoke, bullets,
        // explosions, muzzle flashes) is a top-of-world effect and is drawn in the
        // grouped effects pass below, so none of it splits the rock field's batch.
        enum Item<'a> {
            Rock(&'a world::Rock),
            Rocket(&'a Rocket),
            Vehicle(usize),
        }

        let mut items: Vec<(f32, Item)> = Vec::with_capacity(256);

        for chunk in self.world.chunks.values() {
            for rock in &chunk.rocks {
                if visible(rock.pos) {
                    items.push((y_sort_key(rock.pos, 0.0), Item::Rock(rock)));
                }
            }
        }
        for r in &self.rockets {
            if visible(r.pos) {
                items.push((y_sort_key(r.pos, 0.0), Item::Rocket(r)));
            }
        }
        for i in 0..2 {
            if !self.players[i].is_dead() {
                items.push((y_sort_key(self.players[i].pos, 0.0), Item::Vehicle(i)));
            }
        }

        items.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));

        for (_, item) in &items {
            match item {
                Item::Rock(r) => rocks.draw(r),
                Item::Rocket(r) => rocket_sprites.draw(r.angle, r.pos),
                Item::Vehicle(i) => draw_vehicle(&self.players[*i], *i, ships),
            }
        }

        // Effects pass — all draw on top of the world sprites, grouped by
        // primitive type so each kind stays in a single batch instead of toggling
        // the bound texture / draw mode per item. Order within the pass also sets
        // the visual layering: filled circles (smoke → explosion sparks/cores →
        // muzzle flashes → bullets), then ring lines, then the explosion sprites
        // on top. Trade-off: bullets no longer occlude behind rocks — fine for
        // fast-moving projectiles and faint smoke.
        for e in &self.explosions {
            if visible(e.pos) {
                e.draw_glow();
            }
        }
        for s in &self.smoke {
            if visible(s.pos) {
                s.draw();
            }
        }
        for rocket in &self.rockets {
            if visible(rocket.pos) {
                rocket_sprites.draw_propellant_glow(rocket.angle, rocket.pos);
            }
        }
        for e in &self.explosions {
            if visible(e.pos) {
                e.draw_fills();
            }
        }
        for p in &self.particles {
            if visible(p.pos) {
                p.draw();
            }
        }
        for b in &self.bullets {
            if visible(b.pos) {
                bullet::draw_bullet(b);
            }
        }
        for e in &self.explosions {
            if visible(e.pos) {
                e.draw_ring();
            }
        }
        for e in &self.explosions {
            if visible(e.pos) {
                e.draw_sprite(expl);
            }
        }

        // Every AI slot's overlay is drawn in each viewport: sprites are placed in
        // absolute iso-screen space, so an AI's annotations land on its own ship
        // wherever it is, and are clipped away when it is off this viewport. That
        // also keeps F3 useful in single-screen mode, where the only viewer is
        // human and would otherwise show nothing.
        if self.show_ai_debug {
            for i in 0..2 {
                self.draw_ai_debug(i);
            }
        }

        let other = 1 - viewer;
        draw_offscreen_indicator(
            self.players[viewer].pos,
            self.players[other].pos,
            other,
            view_size,
        );
    }

    // F3 overlay: visualise an AI slot's decision state — active behavior,
    // sampled target, predicted intercept, obstacle probes, and the final
    // avoidance vector. Drawn in the viewport's world/screen space.
    fn draw_ai_debug(&self, slot: usize) {
        let agent = match self.agents[slot].as_ref() {
            Some(a) if self.controllers[slot].is_ai() => a,
            _ => return,
        };
        let d: &AiDebug = &agent.debug;
        let ship = world_to_screen(self.players[slot].pos);

        // Obstacle probes — faint lines to each threatening rock.
        for (a, b) in &d.probes {
            let sa = world_to_screen(*a);
            let sb = world_to_screen(*b);
            draw_line(sa.x, sa.y, sb.x, sb.y, 1.0, Color::new(1.0, 0.6, 0.1, 0.6));
        }

        // Final avoidance steer vector from the ship.
        if d.avoid.length_squared() > 1e-4 {
            let tip = world_to_screen(self.players[slot].pos + d.avoid.clamp_length_max(3.0));
            let col = if d.avoid_urgent {
                Color::new(1.0, 0.2, 0.2, 0.9)
            } else {
                Color::new(0.3, 1.0, 0.5, 0.8)
            };
            draw_line(ship.x, ship.y, tip.x, tip.y, 2.0, col);
        }

        // Sampled (noisy) target position.
        let st = world_to_screen(d.sampled_target);
        draw_circle_lines(st.x, st.y, 8.0, 1.5, Color::new(1.0, 1.0, 0.3, 0.9));

        // Predicted intercept point (where the AI is actually aiming).
        if d.has_intercept {
            let ip = world_to_screen(d.intercept);
            draw_line(
                ip.x - 6.0,
                ip.y,
                ip.x + 6.0,
                ip.y,
                1.5,
                Color::new(0.4, 0.9, 1.0, 0.9),
            );
            draw_line(
                ip.x,
                ip.y - 6.0,
                ip.x,
                ip.y + 6.0,
                1.5,
                Color::new(0.4, 0.9, 1.0, 0.9),
            );
        }

        // Behavior + fire state label above the ship.
        let label = format!(
            "AI P{} [{}] {}{}{}",
            agent.player_index + 1,
            agent.difficulty.label(),
            d.behavior.label(),
            if d.kiting { " KITED" } else { "" },
            if d.firing { " FIRE" } else { "" }
        );
        draw_text(
            &label,
            ship.x - 40.0,
            ship.y - 34.0,
            16.0,
            Color::new(0.9, 1.0, 0.9, 0.95),
        );
    }

    // HUD is composited over the finished frame, so it is layout-independent:
    // the bar/score columns sit at 25 % and 75 % of the window width either way
    // — centred in each half under split screen, flanking the single view.
    pub fn draw_hud(&self) {
        let hw = screen_width() * 0.5;
        let h = screen_height();
        let bar_w = 120.0_f32;
        draw_hud_bars(&self.players[0], hw * 0.5 - bar_w * 0.5, h - 38.0, 0);
        draw_hud_bars(&self.players[1], hw * 1.5 - bar_w * 0.5, h - 38.0, 1);

        // Persistent score display above each player's bars.
        let score_size = 22.0_f32;
        for (i, &cx) in [hw * 0.5, hw * 1.5].iter().enumerate() {
            let s = self.scores[i].to_string();
            let d = measure_text(&s, None, score_size as u16, 1.0);
            let c = PLAYER_COLORS[i];
            draw_text(&s, cx - d.width * 0.5, h - 46.0, score_size, c);
        }

        // Perf overlay — top-right corner.
        {
            let label = format!("{:.0} fps  {:.2} ms", self.fps_avg, self.draw_ms);
            let font_size = 16.0_f32;
            let d = measure_text(&label, None, font_size as u16, 1.0);
            draw_text(
                &label,
                screen_width() - d.width - 8.0,
                d.height + 6.0,
                font_size,
                Color::new(1.0, 1.0, 1.0, 0.65),
            );
        }

        // Kill-flash: large score numbers that fade out over 2.5 s.
        if self.score_flash_timer > 0.0 {
            let alpha = (self.score_flash_timer / 0.5).min(1.0); // full for first 2s, fade last 0.5s
            let cx = screen_width() * 0.5;
            let cy = screen_height() * 0.5 - 10.0;
            let font_size = 80.0_f32;

            let s0 = self.scores[0].to_string();
            let s1 = self.scores[1].to_string();
            let sep = "  –  ";

            let d0 = measure_text(&s0, None, font_size as u16, 1.0);
            let ds = measure_text(sep, None, font_size as u16, 1.0);
            let d1 = measure_text(&s1, None, font_size as u16, 1.0);
            let total_w = d0.width + ds.width + d1.width;
            let x0 = cx - total_w * 0.5;

            let c0 = Color::new(
                PLAYER_COLORS[0].r,
                PLAYER_COLORS[0].g,
                PLAYER_COLORS[0].b,
                alpha,
            );
            let cs = Color::new(1.0, 1.0, 1.0, alpha * 0.7);
            let c1 = Color::new(
                PLAYER_COLORS[1].r,
                PLAYER_COLORS[1].g,
                PLAYER_COLORS[1].b,
                alpha,
            );

            draw_text(&s0, x0, cy, font_size, c0);
            draw_text(sep, x0 + d0.width, cy, font_size, cs);
            draw_text(&s1, x0 + d0.width + ds.width, cy, font_size, c1);
        }
    }
}

// ── Drawing helpers ───────────────────────────────────────────────────────────

fn draw_vehicle(p: &Player, owner: usize, ships: &ShipSprites) {
    let sc = world_to_screen(p.pos);

    // At < 30 % hull, pulse a red tint over the sprite.
    let hull_frac = p.hull / Player::MAX_HULL;
    let tint = if hull_frac < 0.3 {
        let pulse = (get_time() as f32 * 8.0).sin() * 0.5 + 0.5; // 0..1
        Color::new(1.0, 1.0 - pulse * 0.45, 1.0 - pulse * 0.45, 1.0)
    } else {
        WHITE
    };

    ships.draw(p.angle, owner, sc, tint);

    // Hit-flash ring — only while shield is active.
    if p.hit_flash > 0.0 && p.shield > 0.0 {
        let t = p.hit_flash / Player::FLASH_DURATION;
        let k = FRAC_1_SQRT_2;
        let rx = p.radius * TW * k * 1.7;
        let ry = p.radius * TH * k * 1.7;
        draw_ellipse_lines(
            sc.x,
            sc.y,
            rx,
            ry,
            0.0,
            2.5,
            Color::new(1.0, 1.0, 1.0, t.sqrt() * 0.9),
        );
    }
}

fn draw_hud_bars(p: &Player, x: f32, y: f32, player_idx: usize) {
    let w = 120.0_f32;
    let s = (p.shield / Player::MAX_SHIELD).clamp(0.0, 1.0);
    let h = (p.hull / Player::MAX_HULL).clamp(0.0, 1.0);

    // Dim backgrounds.
    draw_rectangle(x, y, w, 8.0, Color::new(0.0, 0.0, 0.25, 0.7));
    draw_rectangle(x, y + 10.0, w, 8.0, Color::new(0.25, 0.0, 0.0, 0.7));
    // Filled bars.
    draw_rectangle(x, y, w * s, 8.0, SKYBLUE);
    draw_rectangle(x, y + 10.0, w * h, 8.0, RED);
    // Outlines.
    draw_rectangle_lines(x, y, w, 8.0, 1.0, WHITE);
    draw_rectangle_lines(x, y + 10.0, w, 8.0, 1.0, WHITE);

    // Heat gauge — thin bar below shield/hull. Color ramps from normal, to a
    // near-overheat warning, to a pulsing red when the gun is actually locked
    // out, so the three states read distinctly at a glance.
    let heat_y = y + 20.0;
    let heat_h = 6.0;
    let heat_frac = (p.heat / Player::OVERHEAT_THRESHOLD).clamp(0.0, 1.0);
    draw_rectangle(x, heat_y, w, heat_h, Color::new(0.2, 0.15, 0.05, 0.7));
    let heat_color = if p.overheated {
        let pulse = (get_time() as f32 * 6.0).sin() * 0.5 + 0.5;
        Color::new(1.0, 0.1 + pulse * 0.15, 0.05, 1.0)
    } else if heat_frac > 0.75 {
        Color::new(1.0, 0.55, 0.05, 1.0)
    } else {
        Color::new(1.0, 0.85, 0.35, 1.0)
    };
    draw_rectangle(x, heat_y, w * heat_frac, heat_h, heat_color);
    draw_rectangle_lines(
        x,
        heat_y,
        w,
        heat_h,
        1.0,
        if p.overheated { RED } else { WHITE },
    );

    // Missile count icons — drawn below the bars.
    let icon_w = 10.0_f32;
    let icon_h = 14.0_f32;
    let gap = 5.0_f32;
    let total = (icon_w + gap) * Player::MAX_MISSILES as f32 - gap;
    let ix = x + w * 0.5 - total * 0.5;
    let iy = y + 30.0;
    let pc = PLAYER_COLORS[player_idx];
    for i in 0..Player::MAX_MISSILES {
        let mx = ix + i as f32 * (icon_w + gap);
        let filled = i < p.missile_count;
        if filled {
            draw_rectangle(mx, iy, icon_w, icon_h, pc);
            // Nose triangle on top.
            draw_triangle(
                vec2(mx + icon_w * 0.5, iy - 5.0),
                vec2(mx, iy),
                vec2(mx + icon_w, iy),
                pc,
            );
        } else {
            draw_rectangle_lines(
                mx,
                iy,
                icon_w,
                icon_h,
                1.0,
                Color::new(pc.r, pc.g, pc.b, 0.4),
            );
            draw_triangle_lines(
                vec2(mx + icon_w * 0.5, iy - 5.0),
                vec2(mx, iy),
                vec2(mx + icon_w, iy),
                1.0,
                Color::new(pc.r, pc.g, pc.b, 0.4),
            );
        }
    }
}

// ── Off-screen indicator ──────────────────────────────────────────────────────

fn draw_offscreen_indicator(viewer_pos: Vec2, other_pos: Vec2, other_idx: usize, view_size: Vec2) {
    let vw = view_size.x;
    let vh = view_size.y;
    let margin = 28.0_f32;

    let viewer_iso = world_to_screen(viewer_pos);
    let other_iso = world_to_screen(other_pos);
    let screen = other_iso - viewer_iso + vec2(vw * 0.5, vh * 0.5);

    if screen.x >= margin
        && screen.x <= vw - margin
        && screen.y >= margin
        && screen.y <= vh - margin
    {
        return;
    }

    let center = vec2(vw * 0.5, vh * 0.5);
    let dir = (screen - center).normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return;
    }

    let tx = if dir.x > 0.0 {
        (vw - margin - center.x) / dir.x
    } else if dir.x < 0.0 {
        (margin - center.x) / dir.x
    } else {
        f32::INFINITY
    };
    let ty = if dir.y > 0.0 {
        (vh - margin - center.y) / dir.y
    } else if dir.y < 0.0 {
        (margin - center.y) / dir.y
    } else {
        f32::INFINITY
    };
    let clamped = center + dir * tx.min(ty);
    let draw_pos = viewer_iso + clamped - center;

    let s = 12.0_f32;
    let perp = vec2(-dir.y, dir.x);
    let tip = draw_pos + dir * s;
    let bl = draw_pos - dir * (s * 0.3) + perp * (s * 0.7);
    let br = draw_pos - dir * (s * 0.3) - perp * (s * 0.7);
    draw_triangle(tip, bl, br, PLAYER_COLORS[other_idx]);
    draw_triangle_lines(tip, bl, br, 1.5, WHITE);
}

// ── Physics helpers ───────────────────────────────────────────────────────────

const RESTITUTION: f32 = 0.65;

// Returns true if the collision speed exceeded the damage threshold.
fn resolve_players(players: &mut [Player; 2]) -> bool {
    let delta = players[0].pos - players[1].pos;
    let min_dist = players[0].radius + players[1].radius;
    let dist2 = delta.length_squared();
    if dist2 >= min_dist * min_dist || dist2 < 1e-4 {
        return false;
    }
    let dist = dist2.sqrt();
    let n = delta / dist;

    let correction = n * (min_dist - dist) * 0.5;
    players[0].pos += correction;
    players[1].pos -= correction;

    let v0n = players[0].vel.dot(n);
    let v1n = players[1].vel.dot(n);
    if v0n - v1n >= 0.0 {
        return false;
    }

    let closing = v1n - v0n; // > 0 when approaching

    let new_v0n = ((1.0 - RESTITUTION) * v0n + (1.0 + RESTITUTION) * v1n) * 0.5;
    let new_v1n = ((1.0 - RESTITUTION) * v1n + (1.0 + RESTITUTION) * v0n) * 0.5;

    players[0].hit_flash = Player::FLASH_DURATION;
    players[1].hit_flash = Player::FLASH_DURATION;

    let had_collision = closing > Player::COLLISION_MIN_SPEED;
    if had_collision {
        let dmg = (closing - Player::COLLISION_MIN_SPEED) * Player::COLLISION_DAMAGE_SCALE;
        if players[0].shield <= 0.0 {
            players[0].apply_damage(dmg);
        }
        if players[1].shield <= 0.0 {
            players[1].apply_damage(dmg);
        }
    }

    players[0].vel += n * (new_v0n - v0n);
    players[1].vel += n * (new_v1n - v1n);

    had_collision
}

// ── Input ─────────────────────────────────────────────────────────────────────

fn axis(neg: bool, pos: bool) -> f32 {
    (pos as i32 - neg as i32) as f32
}

// See the help screen table in main.rs for the full mapping.
fn read_input_p1() -> PlayerInput {
    PlayerInput {
        throttle: axis(is_key_down(KeyCode::S), is_key_down(KeyCode::W)),
        turn: axis(is_key_down(KeyCode::A), is_key_down(KeyCode::D)),
        strafe: axis(is_key_down(KeyCode::R), is_key_down(KeyCode::T)),
        fire: is_key_down(KeyCode::LeftShift),
        fire_rocket: is_key_pressed(KeyCode::Q),
        // Overwritten by the caller with the opponent's position.
        turret_target: Vec2::ZERO,
    }
}

fn read_input_p2() -> PlayerInput {
    PlayerInput {
        throttle: axis(is_key_down(KeyCode::Down), is_key_down(KeyCode::Up)),
        turn: axis(is_key_down(KeyCode::Left), is_key_down(KeyCode::Right)),
        strafe: axis(is_key_down(KeyCode::Comma), is_key_down(KeyCode::Period)),
        // Slash doubles up on Right Shift — sits right next to it, for a
        // two-handed grip where the strafe hand's other digit can fire too.
        fire: is_key_down(KeyCode::RightShift) || is_key_down(KeyCode::Slash),
        fire_rocket: is_key_pressed(KeyCode::Enter),
        // Overwritten by the caller with the opponent's position.
        turret_target: Vec2::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiDifficulty;

    fn layout_for(kinds: [ControllerKind; 2]) -> Layout {
        GameState::new(kinds, GameSpeed::Normal).layout()
    }

    #[test]
    fn one_human_gets_the_whole_window() {
        let ai = ControllerKind::Ai(AiDifficulty::Normal);
        assert!(layout_for([ControllerKind::Human, ai]) == Layout::Single { viewer: 0 });
        assert!(layout_for([ai, ControllerKind::Human]) == Layout::Single { viewer: 1 });
    }

    #[test]
    fn two_humans_or_no_humans_stay_split() {
        let ai = ControllerKind::Ai(AiDifficulty::Hard);
        assert!(layout_for([ControllerKind::Human, ControllerKind::Human]) == Layout::Split);
        assert!(layout_for([ai, ai]) == Layout::Split);
    }

    #[test]
    fn single_viewport_is_twice_as_wide_as_a_split_half() {
        assert_eq!(Layout::Split.view_size(), vec2(VW as f32, VH as f32));
        assert_eq!(
            Layout::Single { viewer: 0 }.view_size(),
            vec2(VW as f32 * 2.0, VH as f32),
        );
    }
}
