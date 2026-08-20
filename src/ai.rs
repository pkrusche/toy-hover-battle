//! Behavior-tree AI opponents.
//!
//! An [`AiAgent`] evaluates a small behavior tree each frame and emits an
//! ordinary [`PlayerInput`], so AI and human players share the exact same
//! movement, cooldown, collision, projectile, damage, and respawn systems in
//! `game.rs`. The agent never mutates game state — `tick` reads immutable
//! [`PlayerView`] snapshots plus the world and returns controls.
//!
//! Design notes:
//! - Global opponent awareness, but the AI reacts to *delayed, noisy* samples
//!   (see [`Tuning::reaction`] / [`Tuning::aim_error`]) so tracking is never
//!   perfect.
//! - The hull turns at a limited rate and has a limited lateral thruster
//!   ([`PlayerInput::strafe`]) — but the gun also sits on a turret with
//!   limited traverse off the nose ([`Player::turret_angle`]), so facing and
//!   aim are decoupled. The movement branch owns hull facing and strafe
//!   (turning to face its steering vector and thrusting laterally to close
//!   the gap while the turn catches up, see `apply_move`) and the attack
//!   branch owns the turret (`turret_target`), which is what makes
//!   `Tuning::strafe` real instead of a radial shuffle.
//! - Obstacle avoidance is deliberately local/reactive (corridor sweep +
//!   repulsion/tangential steering); no global pathfinding.

use std::f32::consts::{PI, TAU};

use ::rand::{RngExt, SeedableRng};
use macroquad::prelude::*;
use rand_xoshiro::Xoshiro256PlusPlus;

use crate::bullet::{Bullet, Rocket};
use crate::iso::world_angle_to_screen_angle;
use crate::player::{Player, PlayerInput};
use crate::world::World;

// ── Public difficulty / controller types ───────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiDifficulty {
    Easy,
    Normal,
    Hard,
}

impl AiDifficulty {
    pub fn label(self) -> &'static str {
        match self {
            AiDifficulty::Easy => "Easy",
            AiDifficulty::Normal => "Normal",
            AiDifficulty::Hard => "Hard",
        }
    }
}

/// Who controls a given player slot. Passed through `GameState::new`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControllerKind {
    Human,
    Ai(AiDifficulty),
}

impl ControllerKind {
    pub fn label(self) -> &'static str {
        match self {
            ControllerKind::Human => "Human",
            ControllerKind::Ai(d) => d.label(),
        }
    }

    /// Cycle Human → Easy → Normal → Hard → Human, for the setup screen.
    pub fn next(self) -> Self {
        match self {
            ControllerKind::Human => ControllerKind::Ai(AiDifficulty::Easy),
            ControllerKind::Ai(AiDifficulty::Easy) => ControllerKind::Ai(AiDifficulty::Normal),
            ControllerKind::Ai(AiDifficulty::Normal) => ControllerKind::Ai(AiDifficulty::Hard),
            ControllerKind::Ai(AiDifficulty::Hard) => ControllerKind::Human,
        }
    }

    /// Reverse of [`next`], for the setup screen's left arrow.
    pub fn prev(self) -> Self {
        match self {
            ControllerKind::Human => ControllerKind::Ai(AiDifficulty::Hard),
            ControllerKind::Ai(AiDifficulty::Easy) => ControllerKind::Human,
            ControllerKind::Ai(AiDifficulty::Normal) => ControllerKind::Ai(AiDifficulty::Easy),
            ControllerKind::Ai(AiDifficulty::Hard) => ControllerKind::Ai(AiDifficulty::Normal),
        }
    }

    pub fn is_ai(self) -> bool {
        matches!(self, ControllerKind::Ai(_))
    }
}

// ── Behavior-tree node results ──────────────────────────────────────────────

/// Explicit node result for the hand-written behavior tree. `Success` means the
/// node took responsibility (a selector stops here); `Failure` lets a selector
/// fall through to the next child; `Running` is an ongoing action that also
/// stops selector fall-through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeResult {
    Success,
    Failure,
    Running,
}

/// The currently active movement behavior — surfaced for the F3 debug overlay.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Behavior {
    Inactive,
    Loiter,
    Avoid,
    Pursue,
    Orbit,
    /// Coasting to swing the nose (and the gun with it) onto the target — see
    /// `PIVOT_ALIGN`.
    Pivot,
    /// Giving ground after taking damage — see [`Tuning::withdraw`].
    Withdraw,
}

impl Behavior {
    pub fn label(self) -> &'static str {
        match self {
            Behavior::Inactive => "Inactive",
            Behavior::Loiter => "Loiter",
            Behavior::Avoid => "Avoid",
            Behavior::Pursue => "Pursue",
            Behavior::Orbit => "Orbit",
            Behavior::Pivot => "Pivot",
            Behavior::Withdraw => "Withdraw",
        }
    }
}

// ── Player snapshot ─────────────────────────────────────────────────────────

/// Immutable snapshot of a player, taken before any agent ticks so that
/// evaluating two AI agents is independent of tick order.
#[derive(Clone, Copy, Debug)]
pub struct PlayerView {
    pub pos: Vec2,
    pub vel: Vec2,
    pub angle: f32,
    /// Current gun facing (see `Player::turret_angle`) — one frame stale,
    /// like every other field here, so the gun-fire cone check accounts for
    /// the turret's own traverse lag rather than an instantaneous target.
    pub turret_angle: f32,
    pub radius: f32,
    /// Remaining damage pool (hull + shield). The agent only reads *drops* in
    /// this, as the trigger for withdrawing (see [`Tuning::withdraw`]).
    pub hp: f32,
    pub dead: bool,
}

impl PlayerView {
    pub fn of(p: &Player) -> Self {
        Self {
            pos: p.pos,
            vel: p.vel,
            angle: p.angle,
            turret_angle: p.turret_angle,
            radius: p.radius,
            hp: p.hull + p.shield,
            dead: p.is_dead(),
        }
    }
}

// ── Difficulty tuning ───────────────────────────────────────────────────────

/// All per-difficulty tuning lives here — no config files, no extra deps.
#[derive(Clone, Copy)]
pub struct Tuning {
    /// Seconds between target re-samples (reaction delay).
    pub reaction: f32,
    /// Bounded positional aim error, tiles.
    pub aim_error: f32,
    /// Half-angle of the gun firing cone, radians.
    pub fire_cone: f32,
    /// Orbit strafe strength, 0..1.
    pub strafe: f32,
    /// Seconds between orbit-direction changes.
    pub orbit_flip: f32,
    /// Preferred combat distance, tiles.
    pub preferred: f32,
    /// Half-width of the combat band around `preferred`, tiles.
    pub band: f32,
    /// Inner safety distance — reverse when closer than this, tiles.
    pub inner: f32,
    /// Per-attempt probability of actually launching an eligible rocket.
    pub rocket_chance: f32,
    /// Minimum seconds between rocket launch attempts.
    pub rocket_interval: f32,
    /// Overall aggression, 0..1. Scales how hard the AI presses under fire and
    /// against a kiter (extra strafe/juke and rocket pressure). Hard = 1.0.
    pub aggression: f32,
    /// Press the attack: hold the hull's nose — and therefore the gun, which
    /// only traverses `Player::TURRET_MAX` off it — on the opponent, fight at
    /// knife range, and give ground only after taking damage. All three
    /// difficulties press now; only `withdraw_hp_frac` (how much of a beating
    /// each is willing to take before backing off) tells them apart. The old
    /// stand-off orbit, which frequently pointed its flank at the target, is
    /// unused but kept as the `else` arm for a future non-pressing difficulty.
    pub press: bool,
    /// Seconds of giving ground after taking a hit, before pressing again.
    /// Only read when `press` is set.
    pub withdraw: f32,
    /// Fraction of the max hp pool (hull + shield) at or below which a hit
    /// earns a withdrawal. Only read when `press` is set. All three
    /// difficulties press through early damage and only back off once
    /// genuinely low, per [`Tuning::withdraw`] — Easy's threshold is highest
    /// (backs off soonest), Hard's is lowest (gives up the least ground).
    pub withdraw_hp_frac: f32,
    /// Seconds the gunfire leaf must have been blocked *specifically by a
    /// rock* (aligned, in range, otherwise clear shot) before the AI gives up
    /// repositioning and fires on the blocking rock instead — see
    /// `node_attack`'s rock-pressing leaf. A momentary occlusion during normal
    /// maneuvering resets the timer well under this, so only a sustained pin
    /// triggers it. Hard presses cover soonest, Easy is most patient.
    pub press_cover_delay: f32,
}

impl Tuning {
    pub fn for_difficulty(d: AiDifficulty) -> Self {
        match d {
            AiDifficulty::Easy => Tuning {
                reaction: 0.45,
                aim_error: 3.4,
                fire_cone: 9.0_f32.to_radians(),
                strafe: 0.4,
                orbit_flip: 4.0,
                preferred: 6.0,
                band: 1.5,
                inner: 3.5,
                rocket_chance: 0.08,
                rocket_interval: 2.0,
                aggression: 0.35,
                press: true,
                withdraw: 2.5,
                withdraw_hp_frac: 0.5,
                press_cover_delay: 1.0,
            },
            AiDifficulty::Normal => Tuning {
                reaction: 0.22,
                aim_error: 1.15,
                fire_cone: 5.0_f32.to_radians(),
                strafe: 0.65,
                orbit_flip: 2.8,
                preferred: 5.5,
                band: 1.5,
                inner: 3.5,
                rocket_chance: 0.24,
                rocket_interval: 1.4,
                aggression: 0.6,
                press: true,
                withdraw: 2.0,
                withdraw_hp_frac: 0.3,
                press_cover_delay: 0.2,
            },
            AiDifficulty::Hard => Tuning {
                reaction: 0.10,
                aim_error: 0.25,
                fire_cone: 3.0_f32.to_radians(),
                strafe: 1.0,
                orbit_flip: 1.5,
                preferred: 5.0,
                band: 1.5,
                inner: 3.0,
                rocket_chance: 0.8,
                rocket_interval: 0.9,
                aggression: 1.0,
                press: true,
                withdraw: 1.1,
                withdraw_hp_frac: 0.15,
                press_cover_delay: 0.1,
            },
        }
    }
}

// Gun range beyond which the AI won't bother firing bullets (tiles). Kept under
// BULLET_REACH with margin so a *led* shot (aimed ahead of a moving target)
// still lands before the bullet expires, and never past a human's on-screen
// radius — the iso viewport shows only ~7.5 tiles along the main axes (as
// little as ~5 along the wide diagonal). Both bounds land it around 7.5.
const GUN_RANGE: f32 = 7.5;
// How far out `rock_press_timer` still remembers a pin. A ship working a piece
// of cover slides in and out of `GUN_RANGE` constantly while hunting a flank
// (measured: in range under half the time), so zeroing the timer the moment
// the opponent drifts out of gun range makes the slower difficulties' press
// delays unreachable — Easy's timer peaked at 1.6s against its 2.5s delay.
// Inside this band the timer holds instead of resetting; past it the agent has
// genuinely disengaged and the pin is forgotten.
const PRESS_MEMORY_RANGE: f32 = GUN_RANGE * 1.5;
// Rocket eligibility window (tiles) and alignment tolerance (radians).
const ROCKET_MIN: f32 = 4.5;
const ROCKET_MAX: f32 = 12.0;
const ROCKET_ALIGN: f32 = 10.0_f32 * PI / 180.0;

/// Linear positional aim error per tile of range — a fixed *angular* sloppiness
/// so the AI doesn't grow sharper the farther it shoots.
pub const AIM_DIST_NOISE: f32 = 0.07;
/// Range (tiles) past which a target is off a human's screen. Beyond it the aim
/// error grows *quadratically* (see `range_scale`) so the AI can't snipe
/// accurately at distances the player can't even see — the view radius in this
/// iso projection is only ~8-12 tiles — it degrades off-screen rocket aim.
const AIM_OFFSCREEN_DIST: f32 = 11.0;
/// Quadratic aim-error growth per tile beyond `AIM_OFFSCREEN_DIST`.
const AIM_OFFSCREEN_NOISE: f32 = 0.13;

/// Multiplier on the base per-difficulty `aim_error` as a function of range:
/// linear angular sloppiness, plus a super-linear penalty once the target is
/// off-screen so long-range shots scatter hard. With `GUN_RANGE` now ~12 this
/// mainly degrades off-screen *rocket* aim (`ROCKET_MAX` = 12).
pub fn range_scale(dist: f32) -> f32 {
    let over = (dist - AIM_OFFSCREEN_DIST).max(0.0);
    1.0 + dist * AIM_DIST_NOISE + (over * AIM_OFFSCREEN_NOISE).powi(2)
}

/// Half-angle of the cone in which the opponent is considered to be aiming at
/// us (i.e. able to shoot us), for kite detection.
const THREAT_CONE: f32 = 40.0_f32 * PI / 180.0;
/// Max range at which we treat the opponent's facing as a threat — just under
/// `BULLET_REACH`, since beyond that the opponent's bullets can't reach us
/// anyway. Sits a little past `GUN_RANGE` so the AI starts dodging an aiming
/// enemy before it's itself in firing range.
const THREAT_RANGE: f32 = 10.0;
/// Opponent closing/receding speed (tiles/s, along the line to us) above which
/// they count as actively backing away — the kite signature.
const RECEDE_SPEED: f32 = 1.0;
/// Rate (tiles/s) at which the *gap* has to be opening to count as losing
/// ground, even when the opponent's own radial speed is under `RECEDE_SPEED`.
/// Catches the case the absolute test misses: we have coasted to a stop inside
/// the band and they are merely drifting out of our reach.
const GAP_OPEN_SPEED: f32 = 0.35;
/// Seconds the kite flag is held after the signature drops. A jinking opponent
/// flicks in and out of `THREAT_CONE` every few frames; without this the sprint
/// and the rocket pressure strobe on and off with it.
const KITE_HOLD: f32 = 0.6;

/// Steady-state speed (tiles/s) per unit of throttle: thrust balances the hull's
/// linear damping at `ACCEL / DAMPING`, so ~0.7 throttle already saturates
/// `MAX_SPEED`. Converts a wanted radial speed into the throttle that holds it.
const SPEED_PER_THROTTLE: f32 = Player::ACCEL / Player::DAMPING;
/// Where inside the combat band the shuttle setpoints sit, as a fraction of the
/// band half-width. Kept under 1.0 so arriving at an edge doesn't tip the
/// movement selector into the pursue/inner arms.
const BAND_EDGE: f32 = 0.85;
/// Distance (tiles) at which a shuttle setpoint counts as reached and the ship
/// turns around. Loose enough that it flips while still carrying speed.
const BAND_ARRIVE: f32 = 0.4;

/// How far off the bearing to the opponent a *pressing* hull may point. Kept a
/// margin *inside* `Player::TURRET_MAX` so the gun not only reaches the target
/// but keeps traverse in hand for lead and for the target's own motion — at the
/// stop itself the turret is pinned and the 3–5° fire cone is never satisfied.
/// The plain orbit heading (radial + full tangent) sits ~35–45° off, well past
/// the stop, which is why a stand-off orbit barely shoots at all: measured over
/// a 6 s engagement it asks to fire on 7–21 % of frames against this cone's
/// 73–95 %. The strafe survives as the heading offset inside the cone.
const PRESS_CONE: f32 = Player::TURRET_MAX - 8.0 * PI / 180.0;
/// Bearing error (radians) past which thrusting along the nose no longer takes
/// the ship where it wants to go, so it stops thrusting and lets the hull swing
/// around first: the pressing pivot (turn rate scales down with speed via
/// `Player::TURN_RATE_SPEED_PENALTY`, so bleeding speed buys turn authority —
/// pivot, then attack) and the avoidance brake both use it.
const THRUST_ALIGN: f32 = 45.0 * PI / 180.0;
/// Seconds an urgent dodge stays committed after the corridor reads clear. The
/// sweep is taken along the blended travel direction, so the instant the hull
/// swings onto the escape heading the rock leaves the corridor, the flag drops,
/// the orbit arm steers straight back into it, and the flag raises again: the
/// ship chatters between two opposite headings and never actually leaves. This
/// makes the dodge a maneuver rather than a one-frame twitch.
const AVOID_COMMIT: f32 = 0.35;
/// Speed (tiles/s) under which a ship that is asking for throttle counts as
/// going nowhere, and seconds of that before it declares itself wedged. Local
/// steering has minima — a pocket between two rocks, a face the orbit arm keeps
/// driving into — that no amount of steer blending resolves, and there is no
/// global pathfinding here to notice. The tell is the outcome, not the geometry.
const STUCK_SPEED: f32 = 0.6;
const STUCK_TIME: f32 = 0.7;
/// Seconds spent backing out along a fresh line once wedged. At the capped
/// reverse speed this is a couple of tiles — enough to be clear of the pocket
/// and pointed somewhere new.
const UNSTICK_TIME: f32 = 0.6;
/// Speed (tiles/s) below which cutting throttle accomplishes nothing: there is
/// no momentum left to bleed and holding at zero only strands the ship. Both
/// throttle-cutting rules above are gated on it — without the gate a ship that
/// has already stopped keeps deciding not to move, which is how an AI ends up
/// parked against a rock forever.
const COAST_MIN_SPEED: f32 = 1.5;
/// Fraction of `Tuning::inner` at which a pressing ship still breaks off. It is
/// pure contact avoidance at this range — well inside the stand-off ring, but
/// still clear of the ~1.3 tiles at which two hulls touch.
const PRESS_INNER_FRAC: f32 = 0.75;

// ── Debug snapshot for the F3 overlay ───────────────────────────────────────

#[derive(Clone)]
pub struct AiDebug {
    pub behavior: Behavior,
    pub sampled_target: Vec2,
    pub intercept: Vec2,
    pub has_intercept: bool,
    pub firing: bool,
    /// Kite flag as the movement branch last saw it (incl. `KITE_HOLD`).
    pub kiting: bool,
    /// World-space avoidance steer, anchored at the ship position.
    pub avoid: Vec2,
    pub avoid_urgent: bool,
    /// Obstacle probe segments (world space) considered this frame.
    pub probes: Vec<(Vec2, Vec2)>,
}

impl Default for AiDebug {
    fn default() -> Self {
        Self {
            behavior: Behavior::Inactive,
            sampled_target: Vec2::ZERO,
            intercept: Vec2::ZERO,
            has_intercept: false,
            firing: false,
            kiting: false,
            avoid: Vec2::ZERO,
            avoid_urgent: false,
            probes: Vec::new(),
        }
    }
}

// ── The agent ───────────────────────────────────────────────────────────────

pub struct AiAgent {
    pub player_index: usize,
    pub difficulty: AiDifficulty,
    tuning: Tuning,
    rng: Xoshiro256PlusPlus,

    // Target sampling / aim noise.
    sample_timer: f32,
    sampled_pos: Vec2,
    sampled_vel: Vec2,
    aim_error: Vec2,
    sampled: bool,

    // Orbit steering.
    orbit_sign: f32,
    orbit_timer: f32,
    // Which edge of the combat band the ship is currently running to: +1 outer,
    // −1 inner. Flips on arrival (see the band arm of `node_movement`).
    band_sign: f32,

    // Rocket pacing.
    rocket_timer: f32,

    // Rock-pressing: seconds the gunfire leaf has been blocked specifically by
    // a rock (see `Tuning::press_cover_delay`); reset whenever a clear shot
    // exists or the blocker changes.
    rock_press_timer: f32,

    // Withdrawal: pressing difficulties give ground only once they have been
    // hurt. `last_hp` is the previous frame's pool, `None` across a respawn so
    // the refill isn't misread; `withdraw_timer` is the remaining hold.
    last_hp: Option<f32>,
    withdraw_timer: f32,

    // Set by the movement branch each tick when the opponent is kiting us
    // (facing + receding) — read by the attack branch to prioritise rockets.
    kiting: bool,
    // Remaining hold on `kiting` (see KITE_HOLD), so the flag doesn't strobe.
    kite_hold: f32,

    // Avoidance hysteresis: `avoid_side` holds the lateral side of the steer,
    // `urgent_hold`/`escape_dir` hold the whole dodge together (AVOID_COMMIT).
    avoid_side: f32,
    avoid_hold: f32,
    urgent_hold: f32,
    escape_dir: Vec2,
    // Wedged-ship detection: how long we have been asking for throttle without
    // moving, and the remaining back-out maneuver it triggers.
    stuck_timer: f32,
    unstick_timer: f32,
    unstick_turn: f32,

    // Loiter wander.
    loiter_dir: Vec2,
    loiter_timer: f32,

    pub debug: AiDebug,
}

impl AiAgent {
    /// Create a deterministic per-agent agent. `seed` fully determines the
    /// timing/noise stream, so a fixed seed reproduces the same behavior.
    pub fn new(player_index: usize, difficulty: AiDifficulty, seed: u64) -> Self {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(
            seed ^ (player_index as u64).wrapping_mul(0x9E3779B97F4A7C15),
        );
        let orbit_sign = if rng.random::<bool>() { 1.0 } else { -1.0 };
        let tuning = Tuning::for_difficulty(difficulty);
        Self {
            player_index,
            difficulty,
            tuning,
            rng,
            sample_timer: 0.0,
            sampled_pos: Vec2::ZERO,
            sampled_vel: Vec2::ZERO,
            aim_error: Vec2::ZERO,
            sampled: false,
            orbit_sign,
            orbit_timer: tuning.orbit_flip,
            band_sign: -orbit_sign, // arbitrary; the first arrival flips it anyway
            rocket_timer: tuning.rocket_interval,
            rock_press_timer: 0.0,
            last_hp: None,
            withdraw_timer: 0.0,
            kiting: false,
            kite_hold: 0.0,
            avoid_side: 1.0,
            avoid_hold: 0.0,
            urgent_hold: 0.0,
            escape_dir: Vec2::ZERO,
            stuck_timer: 0.0,
            unstick_timer: 0.0,
            unstick_turn: 1.0,
            loiter_dir: vec2(1.0, 0.0),
            loiter_timer: 0.0,
            debug: AiDebug::default(),
        }
    }

    /// Evaluate the behavior tree and return the controls for this frame.
    /// Reads only snapshots — never mutates game state.
    pub fn tick(&mut self, me: PlayerView, opp: PlayerView, world: &World, dt: f32) -> PlayerInput {
        let mut input = PlayerInput::default();
        let mut probes: Vec<(Vec2, Vec2)> = Vec::new();
        self.debug.firing = false;
        self.debug.has_intercept = false;
        // Nobody to be kited by unless the engagement branch says otherwise.
        if me.dead || opp.dead {
            self.kiting = false;
            self.kite_hold = 0.0;
            self.debug.kiting = false;
        }

        self.track_damage(me, dt);

        // ── Root selector ───────────────────────────────────────────────
        // 1) inactive while dead; 2) loiter while opponent is dead;
        // 3) otherwise acquire a target and run movement + attack together.
        if me.dead {
            self.debug.behavior = Behavior::Inactive;
            self.debug.probes = probes;
            return input;
        }

        if opp.dead {
            self.node_loiter(me, world, dt, &mut input, &mut probes);
            self.debug.behavior = Behavior::Loiter;
            self.debug.probes = probes;
            return input;
        }

        // Acquire (sample) the target on the reaction cadence.
        self.sample_target(me, opp, dt);

        // Movement and attack branches run together (parallel): movement sets
        // throttle/strafe, attack sets turn + fire/fire_rocket.
        self.node_movement(me, opp, world, dt, &mut input, &mut probes);
        self.node_attack(me, opp, world, dt, &mut input);

        self.debug.probes = probes;
        input
    }

    // ── Damage bookkeeping ─────────────────────────────────────────────
    //
    // A pressing agent (see `Tuning::press`) does not disengage on geometry
    // alone — it holds the fight until it is actually hurt, then gives ground
    // for `Tuning::withdraw` seconds. Only *drops* in the pool count: shield
    // recharge and the respawn refill must not read as damage. The drop must
    // also bring the pool down to `Tuning::withdraw_hp_frac` of max before it
    // counts — all three difficulties keep pressing through early damage and
    // only back off once genuinely hurt, scaled by `withdraw_hp_frac`.
    fn track_damage(&mut self, me: PlayerView, dt: f32) {
        self.withdraw_timer = (self.withdraw_timer - dt).max(0.0);
        if me.dead {
            self.withdraw_timer = 0.0;
            self.last_hp = None;
            return;
        }
        if let Some(prev) = self.last_hp {
            let max_hp = Player::MAX_HULL + Player::MAX_SHIELD;
            if self.tuning.press
                && me.hp < prev - 1e-3
                && me.hp <= self.tuning.withdraw_hp_frac * max_hp
            {
                self.withdraw_timer = self.tuning.withdraw;
                // Break outward on the next band leg rather than finishing the
                // inbound one we were on when the hit landed.
                self.band_sign = 1.0;
            }
        }
        self.last_hp = Some(me.hp);
    }

    // ── Target sampling ────────────────────────────────────────────────
    //
    // Re-sample the opponent only every `reaction` seconds and hold a bounded
    // random positional error between samples, so aim lags and is never pixel
    // perfect.
    fn sample_target(&mut self, me: PlayerView, opp: PlayerView, dt: f32) {
        self.sample_timer -= dt;
        if !self.sampled || self.sample_timer <= 0.0 {
            self.sample_timer = self.tuning.reaction;
            self.sampled_pos = opp.pos;
            self.sampled_vel = opp.vel;
            // Uniform in a disk whose radius grows with range, so distant aim is
            // proportionally noisier and off-screen aim is far worse (see
            // range_scale). Bounded per sample.
            let dist = (opp.pos - me.pos).length();
            let ang = self.rng.random::<f32>() * TAU;
            let mag = self.tuning.aim_error * range_scale(dist) * self.rng.random::<f32>().sqrt();
            self.aim_error = vec2(ang.cos(), ang.sin()) * mag;
            self.sampled = true;
        }
        self.debug.sampled_target = self.sampled_pos + self.aim_error;
    }

    // ── Movement branch (selector) ─────────────────────────────────────
    //
    // Urgent obstacle avoidance is selected before pursuit/orbit steering.
    fn node_movement(
        &mut self,
        me: PlayerView,
        opp: PlayerView,
        world: &World,
        dt: f32,
        input: &mut PlayerInput,
        probes: &mut Vec<(Vec2, Vec2)>,
    ) -> NodeResult {
        // Desired orbit/pursuit movement (world space).
        let to_opp = opp.pos - me.pos;
        let dist = to_opp.length();
        let radial_dir = to_opp.normalize_or_zero();
        let tangent_dir = vec2(-radial_dir.y, radial_dir.x) * self.orbit_sign;
        let t = self.tuning;

        // Kite detection: the opponent is facing us (so they can fire on us) and
        // is actively receding — the "back up and shoot the charger" pattern.
        // Since both ships share a max speed, a straight head-on chase is a
        // stalemate, so we respond by weaving to dodge, sprinting to close, and
        // (in the attack branch) spending rockets to force them to break off.
        let opp_to_me = -to_opp;
        let opp_aim = wrap_pi(opp_to_me.y.atan2(opp_to_me.x) - opp.angle).abs();
        let threatened = opp_aim < THREAT_CONE && dist < THREAT_RANGE;
        // Radial rates, positive = "moving away from us". `gap_rate` folds in our
        // own motion, so a slow backpedal that we are failing to answer still
        // reads as losing ground — the absolute test alone lets an opponent
        // drift out from under a stalled AI without ever tripping the flag. The
        // `opp_radial > 0.0` guard keeps the gap-rate path from firing while
        // *we* are the one reversing (out of the inner ring) at a static target.
        let opp_radial = opp.vel.dot(radial_dir);
        let gap_rate = opp_radial - me.vel.dot(radial_dir);
        let receding = opp_radial > RECEDE_SPEED || (opp_radial > 0.0 && gap_rate > GAP_OPEN_SPEED);
        let kiting_now = threatened && receding && dist > t.inner;
        self.kite_hold = if kiting_now {
            KITE_HOLD
        } else {
            (self.kite_hold - dt).max(0.0)
        };
        self.kiting = self.kite_hold > 0.0;
        self.debug.kiting = self.kiting;

        // Flip orbit direction periodically (shorter interval = harder). Under
        // threat we juke roughly twice as often to spoil the opponent's lead.
        self.orbit_timer -= dt;
        if self.orbit_timer <= 0.0 {
            self.orbit_timer = if threatened {
                t.orbit_flip * 0.5
            } else {
                t.orbit_flip
            };
            if self.rng.random::<f32>() < 0.5 {
                self.orbit_sign = -self.orbit_sign;
            }
        }

        // Pressing: fight nose-on and give ground only once hurt (see
        // `Tuning::press`). `withdrawing` is the earned retreat — for the length
        // of `Tuning::withdraw` after a hit the agent falls back to the cautious
        // stand-off arms below.
        let withdrawing = t.press && self.withdraw_timer > 0.0;
        let pressing = t.press && !withdrawing;
        // While pressing, the break-off ring shrinks to pure contact avoidance:
        // the stand-off distance is a withdrawal, and we have not been hit yet.
        let break_off = if pressing {
            t.inner * PRESS_INNER_FRAC
        } else {
            t.inner
        };

        // Arm order matters: the inner ring wins over everything (the kite flag
        // is *held*, and letting a held sprint outrank it rams the ship into the
        // opponent), and the kite sprint only applies once we have actually lost
        // ground — inside `preferred` the band controller below already keeps
        // pace with a retreat, and sprinting there just fights the reverse arm.
        let (radial, strafe_strength, behavior_hint) = if dist < break_off {
            (-1.0, t.strafe, Behavior::Orbit) // reverse out of the inner ring
        } else if self.kiting && dist > t.preferred && !withdrawing {
            // Sprint to close the gap, weaving just enough to be a hard target
            // without killing the closing speed we need to catch a retreater.
            // Aggression sets how much we juke while closing (Hard weaves most).
            (
                1.0,
                (t.strafe * 0.5 * t.aggression).min(0.5),
                Behavior::Pursue,
            )
        } else if dist > t.preferred + t.band {
            (1.0, t.strafe, Behavior::Pursue) // approach
        } else {
            // Hold the band and orbit; strafe harder while under fire to dodge,
            // scaled by aggression so Easy/Normal weave less frantically.
            let strafe = if threatened {
                (t.strafe * (1.0 + 0.4 * t.aggression)).min(1.0)
            } else {
                t.strafe
            };
            // Shuttle between the band's edges rather than homing on
            // `preferred`. A P term on a *fixed* setpoint is what parked the
            // ship at one range: the error decays to zero, so does the throttle,
            // and damping bleeds off the rest — and the tangential term can't
            // take over, since `apply_move` projects onto the heading and the
            // attack branch pins the nose on the target. Flipping the setpoint
            // on arrival gives a permanent in/out joust instead, always carrying
            // speed and always crossing firing range.
            //
            // Pressing turns that shuttle into a committed pass: dive to just
            // outside the contact ring, then fall back out to `preferred` and no
            // further — the outward leg is ground given away for free, and we
            // have not been hit yet. The long inbound leg is also what keeps
            // real speed on the ship; halving the band around `preferred`
            // instead just made it vibrate in place. While withdrawing the sign
            // is pinned outbound (set when the hit landed) so the retreat is not
            // cut short by an arrival flip.
            let target = if pressing {
                if self.band_sign > 0.0 {
                    t.preferred
                } else {
                    break_off + BAND_ARRIVE
                }
            } else {
                t.preferred + self.band_sign * t.band * BAND_EDGE
            };
            if (dist - target).abs() < BAND_ARRIVE && !withdrawing {
                self.band_sign = -self.band_sign;
            }
            let p = ((dist - target) / t.band).clamp(-0.6, 0.6);
            // Feed-forward on the opponent's radial speed: holding station on a
            // *moving* target needs standing throttle (the hull is damped, so
            // speed tracks throttle, not acceleration), which no error term on
            // its own supplies.
            let ff = (opp_radial / SPEED_PER_THROTTLE).clamp(-0.8, 0.8);
            let hint = if withdrawing {
                Behavior::Withdraw
            } else {
                Behavior::Orbit
            };
            ((p + ff).clamp(-1.0, 1.0), strafe, hint)
        };
        // The heading we want. Pressing decouples it from the radial *effort*:
        // the hull stays inside `PRESS_CONE` of the bearing to the opponent so
        // the turret can always bear, and closing/opening the range rides on the
        // throttle sign below (a reverse, capped by `Player::REVERSE_MAX_SPEED`,
        // rather than turning tail and giving up the shot). Everywhere else the
        // radial term steers, as before.
        let orbit_move = if pressing {
            clamp_toward(
                radial_dir + tangent_dir * strafe_strength,
                radial_dir,
                PRESS_CONE,
            )
        } else {
            radial_dir * radial + tangent_dir * strafe_strength
        };

        // Sweep a corridor along the blend of current velocity and intended
        // heading, then steer clear of threatening rocks.
        let travel =
            (me.vel.normalize_or_zero() + orbit_move.normalize_or_zero()).normalize_or_zero();
        let travel = if travel.length_squared() < 1e-6 {
            vec2(me.angle.cos(), me.angle.sin())
        } else {
            travel
        };
        let look = (me.radius + 1.5 + me.vel.length() * 0.45).min(6.0);
        let (steer, urgent) = self.avoidance(me, world, travel, look, dt, probes);
        self.debug.avoid = steer;

        // Latch the escape heading for `AVOID_COMMIT` so the dodge is seen
        // through instead of being re-decided every frame (see the constant).
        if urgent && steer.length_squared() > 1e-6 {
            self.escape_dir = steer.normalize();
            self.urgent_hold = AVOID_COMMIT;
        } else {
            self.urgent_hold = (self.urgent_hold - dt).max(0.0);
        }
        let escaping = self.urgent_hold > 0.0 && self.escape_dir.length_squared() > 1e-6;
        self.debug.avoid_urgent = escaping;

        // Selector: urgent avoidance overrides orbit; otherwise blend a gentle
        // correction into the orbit movement.
        let (mv, brake, behavior) = if escaping {
            (self.escape_dir * 1.2, true, Behavior::Avoid)
        } else {
            (orbit_move + steer * 0.5, false, behavior_hint)
        };
        self.debug.behavior = behavior;

        apply_move(input, me.angle, mv, dt);
        // Both throttle rules below only *cut* throttle, so both need the same
        // guard: the nose still has to be pointed somewhere thrust would not
        // help, and there has to be speed left worth bleeding. `apply_move` has
        // already aimed the hull at `mv`, so once it swings around, thrust is
        // exactly what carries the ship out — clamping past that point is what
        // leaves an AI sitting against a rock with the throttle shut.
        let off_heading = wrap_pi(mv.y.atan2(mv.x) - me.angle).abs() > THRUST_ALIGN
            && me.vel.length() > COAST_MIN_SPEED;
        if brake {
            // Bleed forward speed when dodging head-on into an obstacle — but
            // only while the nose is still pointed into it.
            if off_heading {
                input.throttle = input.throttle.min(0.0);
            }
        } else if pressing {
            // Pressing throttle: the projection `apply_move` computed is the
            // wrong quantity here — the heading is deliberately offset for the
            // strafe, so the radial effort is applied directly instead. Kill
            // literal strafe too: the nose is intentionally held near the
            // opponent's bearing for the turret, and lateral thrust piled on
            // top of the overridden throttle can push the ship past the
            // avoidance geometry the pressing cone was tuned around, straight
            // into whatever it was routing past.
            input.strafe = 0.0;
            if off_heading {
                // Nose well off where we want it: coast. Turn rate falls with
                // speed (`Player::TURN_RATE_SPEED_PENALTY`), so bleeding speed
                // swings the nose — and the gun — onto the target faster than
                // powering through the turn would. Then attack.
                input.throttle = 0.0;
                self.debug.behavior = Behavior::Pivot;
            } else {
                input.throttle = radial.clamp(-1.0, 1.0);
            }
        }
        self.unstick(me, dt, input);
        NodeResult::Running
    }

    // ── Wedged-ship escape ─────────────────────────────────────────────
    //
    // Highest-priority movement override, applied after every other rule: a
    // ship that is asking for throttle and going nowhere has driven into a
    // local minimum of the reactive steering (see `STUCK_SPEED`), and the way
    // out is to give up on the current line entirely — back off and swing onto
    // a new one. The turn side is drawn from the agent's own RNG, so a second
    // attempt on the same pocket is likely to try the other way around.
    fn unstick(&mut self, me: PlayerView, dt: f32, input: &mut PlayerInput) {
        if self.unstick_timer > 0.0 {
            self.unstick_timer -= dt;
            input.throttle = -1.0;
            input.turn = self.unstick_turn;
            self.debug.behavior = Behavior::Avoid;
            return;
        }
        if input.throttle.abs() > 0.2 && me.vel.length() < STUCK_SPEED {
            self.stuck_timer += dt;
            if self.stuck_timer > STUCK_TIME {
                self.stuck_timer = 0.0;
                self.unstick_timer = UNSTICK_TIME;
                self.unstick_turn = if self.rng.random::<bool>() { 1.0 } else { -1.0 };
            }
        } else {
            self.stuck_timer = (self.stuck_timer - dt).max(0.0);
        }
    }

    // ── Attack branch (selector) ───────────────────────────────────────
    //
    // Safe rocket shot → predictive gunfire → aim-only tracking.
    fn node_attack(
        &mut self,
        me: PlayerView,
        opp: PlayerView,
        world: &World,
        dt: f32,
        input: &mut PlayerInput,
    ) -> NodeResult {
        let to_opp = opp.pos - me.pos;
        let dist = to_opp.length();

        // Predictive aim: lead the opponent for a constant-speed bullet. Bullets
        // travel at a fixed ground speed (SPEED along the nose, independent of
        // the shooter's velocity), so the lead uses the target's ground velocity
        // directly — there's no shooter-velocity term to cancel.
        let target = self.sampled_pos + self.aim_error;
        let rel_pos = target - me.pos;
        let rel_vel = self.sampled_vel;
        let (aim_dir, aim_point) =
            match solve_intercept(rel_pos, rel_vel, Bullet::SPEED, Bullet::LIFE) {
                Some((dir, t)) => {
                    self.debug.has_intercept = true;
                    self.debug.intercept = target + self.sampled_vel * t;
                    (dir, self.debug.intercept)
                }
                None => {
                    // No positive intercept within the bullet's lifetime — aim
                    // straight at the (noisy) current position.
                    let dir = rel_pos.normalize_or_zero();
                    self.debug.intercept = target;
                    (dir, target)
                }
            };

        // Always keep the turret tracking the aim point (aim-only tracking is
        // the fallback leaf that is "Running" every frame). Hull facing is
        // owned by the movement branch; this branch only drives the turret.
        input.turret_target = aim_point;

        // Selector over the attack leaves: safe rocket → predictive gunfire →
        // aim-only tracking. Each leaf returns Success when it takes the shot,
        // Failure when it declines, and the selector falls through on Failure.

        // Rocket leaf: eligible window, aligned to the opponent, clear path,
        // paced by a difficulty-scaled opportunity roll (one-frame pulse).
        self.rocket_timer -= dt;
        let align_opp = wrap_pi(to_opp.y.atan2(to_opp.x) - me.angle).abs();
        // Against a kiting opponent, lean harder on homing rockets — they chase
        // the retreater down and force an evasive break, cracking the stalemate.
        // The boost scales with aggression, so Easy/Normal spend rockets far more
        // sparingly than Hard.
        let (rocket_chance, rocket_interval) = if self.kiting {
            let ag = self.tuning.aggression;
            (
                (self.tuning.rocket_chance * (1.0 + 0.5 * ag)).min(1.0),
                self.tuning.rocket_interval * (1.0 - 0.4 * ag),
            )
        } else {
            (self.tuning.rocket_chance, self.tuning.rocket_interval)
        };
        let rocket = if (ROCKET_MIN..=ROCKET_MAX).contains(&dist)
            && align_opp < ROCKET_ALIGN
            && self.rocket_timer <= 0.0
            && world_segment_clear(world, me.pos, opp.pos, Rocket::RADIUS)
        {
            self.rocket_timer = rocket_interval;
            if self.rng.random::<f32>() < rocket_chance {
                input.fire_rocket = true;
                NodeResult::Success
            } else {
                NodeResult::Failure
            }
        } else {
            NodeResult::Failure
        };
        if rocket == NodeResult::Success {
            return NodeResult::Success;
        }

        // Gunfire leaf: aligned within the firing cone, in range, and the
        // predicted shot segment is clear of rocks (expanded by bullet radius).
        // Checked against the turret's actual current facing, not the hull —
        // the turret has its own traverse lag, so a shot only goes out once it
        // has actually swung onto target.
        let align = wrap_pi(aim_dir.y.atan2(aim_dir.x) - me.turret_angle).abs();
        let in_range = aim_dir.length_squared() > 1e-6 && dist < GUN_RANGE;
        let aligned_and_in_range = in_range && align < self.tuning.fire_cone;
        // Blockage is judged on range alone, deliberately *not* on turret
        // alignment: the turret is always traversing, so gating this on
        // alignment would zero `rock_press_timer` on every swing and the
        // pressing leaf below could effectively never fire while the ship
        // maneuvers — which is exactly when an opponent is behind cover.
        // What matters is whether a rock sits on the line, not whether the
        // gun has finished swinging onto it yet.
        let blocker = if in_range {
            world_segment_blocker(world, me.pos, aim_point, Bullet::RADIUS)
        } else {
            None
        };
        if aligned_and_in_range && blocker.is_none() {
            input.fire = true;
            self.debug.firing = true;
            // A clear shot exists — the opponent isn't pinned behind cover.
            self.rock_press_timer = 0.0;
            return NodeResult::Success;
        }

        // Track how long a rock has been the thing standing between us and the
        // opponent. A clear line resets this just as fast as it accumulates,
        // so a momentary occlusion during normal maneuvering can't trip the
        // rock-pressing leaf below — only a sustained pin does. Drifting out
        // of gun range while still working the same piece of cover holds the
        // timer rather than resetting it (see `PRESS_MEMORY_RANGE`); only
        // disengaging outright forgets the pin.
        if blocker.is_some() {
            self.rock_press_timer += dt;
        } else if in_range || dist > PRESS_MEMORY_RANGE {
            self.rock_press_timer = 0.0;
        }

        // Rock-pressing leaf: once genuinely pinned (blocked for at least
        // `press_cover_delay`), aim the turret at the blocking rock instead of
        // the opponent and fire on it under the same fire_cone/heat/cooldown
        // gating as any other shot (the shared fire gate in `game.rs` applies
        // uniformly regardless of what `turret_target` points at).
        if let Some((rock_pos, _)) = blocker {
            if self.rock_press_timer >= self.tuning.press_cover_delay {
                input.turret_target = rock_pos;
                let to_rock = rock_pos - me.pos;
                let rock_align = wrap_pi(to_rock.y.atan2(to_rock.x) - me.turret_angle).abs();
                if rock_align < self.tuning.fire_cone && to_rock.length() < GUN_RANGE {
                    input.fire = true;
                    self.debug.firing = true;
                    return NodeResult::Success;
                }
                return NodeResult::Running;
            }
        }

        // Aim-only tracking (always running).
        NodeResult::Running
    }

    // ── Loiter (opponent dead) ─────────────────────────────────────────
    fn node_loiter(
        &mut self,
        me: PlayerView,
        world: &World,
        dt: f32,
        input: &mut PlayerInput,
        probes: &mut Vec<(Vec2, Vec2)>,
    ) {
        self.loiter_timer -= dt;
        if self.loiter_timer <= 0.0 {
            self.loiter_timer = 2.0;
            let ang = self.rng.random::<f32>() * TAU;
            self.loiter_dir = vec2(ang.cos(), ang.sin());
        }
        let travel = if me.vel.length_squared() > 0.5 {
            me.vel.normalize()
        } else {
            self.loiter_dir
        };
        let look = (me.radius + 1.5 + me.vel.length() * 0.45).min(6.0);
        let (steer, urgent) = self.avoidance(me, world, travel, look, dt, probes);
        self.debug.avoid = steer;
        self.debug.avoid_urgent = urgent;
        let mv = if urgent {
            normalize_to(steer, 1.0)
        } else {
            self.loiter_dir * 0.4 + steer * 0.5
        };
        // apply_move faces the hull toward `mv` (direction of travel) while loitering.
        apply_move(input, me.angle, mv, dt);
        self.unstick(me, dt, input);
    }

    // ── Local obstacle avoidance ───────────────────────────────────────
    //
    // Corridor sweep + repulsion/tangential steering, with brief side
    // retention (hysteresis) to prevent left/right oscillation.
    fn avoidance(
        &mut self,
        me: PlayerView,
        world: &World,
        travel: Vec2,
        look: f32,
        dt: f32,
        probes: &mut Vec<(Vec2, Vec2)>,
    ) -> (Vec2, bool) {
        // Gather nearby rocks that fall inside the swept corridor for probing.
        let dir = travel.normalize_or_zero();
        if dir.length_squared() > 1e-6 {
            for rock in world.rocks_near(me.pos) {
                let rp = rock.pos - me.pos;
                let forward = rp.dot(dir);
                if forward > 0.0 && forward < look {
                    let lateral = (rp - dir * forward).length();
                    if lateral < me.radius + rock.radius + 0.5 {
                        probes.push((me.pos, rock.pos));
                    }
                }
            }
        }

        let rocks: Vec<(Vec2, f32)> = world
            .rocks_near(me.pos)
            .map(|r| (r.pos, r.radius))
            .collect();
        let (mut steer, urgent) = avoidance_steer(me.pos, me.radius, dir, look, &rocks);

        // Hysteresis: hold the chosen lateral side briefly.
        self.avoid_hold -= dt;
        let left = vec2(-dir.y, dir.x);
        let lat = steer - dir * steer.dot(dir);
        if lat.length_squared() > 1e-4 {
            if self.avoid_hold <= 0.0 {
                self.avoid_side = lat.dot(left).signum();
                if self.avoid_side == 0.0 {
                    self.avoid_side = 1.0;
                }
                self.avoid_hold = 0.35;
            }
            let along = dir * steer.dot(dir);
            steer = along + left * (self.avoid_side * lat.length());
        }
        (steer, urgent)
    }
}

// ── Free helpers (pure, unit-tested) ────────────────────────────────────────

/// Solve constant-velocity bullet interception.
///
/// `rel_pos` = target − shooter, `rel_vel` = target_vel − shooter_vel, `speed`
/// is the bullet's muzzle speed. Returns `(unit aim direction, time-to-hit)`
/// for the smallest positive intercept time within `max_time`, or `None` if no
/// such intercept exists (target unreachable, or only hit beyond the lifetime).
pub fn solve_intercept(
    rel_pos: Vec2,
    rel_vel: Vec2,
    speed: f32,
    max_time: f32,
) -> Option<(Vec2, f32)> {
    // |rel_pos + rel_vel*t| = speed*t  ⇒
    //   (speed² − |rel_vel|²) t² − 2(rel_pos·rel_vel) t − |rel_pos|² = 0
    let a = speed * speed - rel_vel.length_squared();
    let b = -2.0 * rel_pos.dot(rel_vel);
    let c = -rel_pos.length_squared();
    let t = smallest_positive_root(a, b, c)?;
    if t <= 1e-4 || t > max_time {
        return None;
    }
    let aim = (rel_pos + rel_vel * t) / (speed * t);
    Some((aim.normalize_or_zero(), t))
}

fn smallest_positive_root(a: f32, b: f32, c: f32) -> Option<f32> {
    if a.abs() < 1e-6 {
        // Linear: b t + c = 0.
        if b.abs() < 1e-9 {
            return None;
        }
        let t = -c / b;
        return if t > 1e-4 { Some(t) } else { None };
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t1 = (-b - sq) / (2.0 * a);
    let t2 = (-b + sq) / (2.0 * a);
    let mut best = f32::INFINITY;
    for t in [t1, t2] {
        if t > 1e-4 && t < best {
            best = t;
        }
    }
    if best.is_finite() {
        Some(best)
    } else {
        None
    }
}

/// Squared distance from point `c` to segment `[a, b]`.
fn point_segment_dist2(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-9 {
        return (c - a).length_squared();
    }
    let t = ((c - a).dot(ab) / len2).clamp(0.0, 1.0);
    (c - (a + ab * t)).length_squared()
}

/// Finds the first rock blocking segment `[a, b]` (each expanded by
/// `expand`), if any — `(pos, radius)` of the blocker.
pub fn segment_blocker<I>(a: Vec2, b: Vec2, expand: f32, rocks: I) -> Option<(Vec2, f32)>
where
    I: IntoIterator<Item = (Vec2, f32)>,
{
    for (pos, r) in rocks {
        let rr = r + expand;
        if point_segment_dist2(a, b, pos) < rr * rr {
            return Some((pos, r));
        }
    }
    None
}

/// True if the segment `[a, b]` clears every rock, each expanded by `expand`.
pub fn segment_clear<I>(a: Vec2, b: Vec2, expand: f32, rocks: I) -> bool
where
    I: IntoIterator<Item = (Vec2, f32)>,
{
    segment_blocker(a, b, expand, rocks).is_none()
}

/// World wrapper for [`segment_blocker`]. Queries rocks around the segment
/// midpoint — valid because `rocks_near` covers the 3×3 chunk block (≥32 tiles
/// of guaranteed reach in each direction from the query point), so for the
/// short segments the AI tests (< ~30 tiles) all intersecting rocks are found.
fn world_segment_blocker(world: &World, a: Vec2, b: Vec2, expand: f32) -> Option<(Vec2, f32)> {
    let mid = (a + b) * 0.5;
    segment_blocker(
        a,
        b,
        expand,
        world.rocks_near(mid).map(|r| (r.pos, r.radius)),
    )
}

/// World wrapper for [`segment_clear`].
fn world_segment_clear(world: &World, a: Vec2, b: Vec2, expand: f32) -> bool {
    let mid = (a + b) * 0.5;
    segment_clear(
        a,
        b,
        expand,
        world.rocks_near(mid).map(|r| (r.pos, r.radius)),
    )
}

/// Core reactive avoidance steering. Given the ship's position/radius and a
/// unit travel direction, returns `(steer, urgent)` where `steer` is a
/// world-space vector combining repulsion (away from rocks) and tangential
/// steering (around them), and `urgent` flags an imminent collision that should
/// override higher-level movement.
pub fn avoidance_steer(
    pos: Vec2,
    radius: f32,
    dir: Vec2,
    look: f32,
    rocks: &[(Vec2, f32)],
) -> (Vec2, bool) {
    let dir = dir.normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return (Vec2::ZERO, false);
    }
    let left = vec2(-dir.y, dir.x);
    let mut repel = Vec2::ZERO;
    let mut tangent = Vec2::ZERO;
    let mut urgent = false;

    for &(rpos, rr) in rocks {
        let rp = rpos - pos;
        let dist = rp.length();

        // Overlapping / inside the rock — shove straight out, always urgent.
        if dist < radius + rr + 0.05 {
            let away = if dist > 1e-3 { -rp / dist } else { left };
            repel += away * 1.5;
            urgent = true;
            continue;
        }

        let forward = rp.dot(dir);
        if forward <= 0.0 {
            continue; // behind us
        }
        let clearance = radius + rr + 0.5;
        let lateral_vec = rp - dir * forward;
        let lateral = lateral_vec.length();
        if forward < look && lateral < clearance {
            let closeness = 1.0 - forward / look; // 0 (far) .. 1 (close)
            repel += (-rp).normalize_or_zero() * closeness;
            // Steer to the side opposite the rock's lateral offset. Head-on
            // (lateral ≈ 0) resolves deterministically to one side.
            let side = if lateral_vec.dot(left) >= 0.0 {
                -1.0
            } else {
                1.0
            };
            tangent += left * (side * (0.5 + closeness));
            if forward < clearance + 0.7 {
                urgent = true;
            }
        }
    }

    (repel * 0.7 + tangent, urgent)
}

/// Turn-input needed to face `target_world_angle`, computed in *screen* space
/// (turning is applied in screen space in `Player::update`, see `iso.rs`).
fn turn_toward(current_world_angle: f32, target_world_angle: f32, dt: f32) -> f32 {
    let cur = world_angle_to_screen_angle(current_world_angle);
    let des = world_angle_to_screen_angle(target_world_angle);
    let err = wrap_pi(des - cur);
    if dt > 1e-5 {
        (err / (Player::TURN_RATE * dt)).clamp(-1.0, 1.0)
    } else {
        err.clamp(-1.0, 1.0)
    }
}

/// Turn the hull to face `mv` and project it onto the resulting forward/left
/// axes for throttle and strafe. The turret (driven separately by the attack
/// branch, see `turret_target`) is what keeps the gun on target — this is
/// what makes `Tuning::strafe` real instead of being silently dropped: a ship
/// can face its orbit direction and thrust across the enemy's face. Turning
/// is rate-limited, so `mv`'s lateral component still has to fight past the
/// hull's own heading each frame — literal strafe thrust is what lets the
/// ship actually cover that gap instead of waiting on the turn.
fn apply_move(input: &mut PlayerInput, angle: f32, mv: Vec2, dt: f32) {
    if mv.length_squared() > 1e-6 {
        input.turn = turn_toward(angle, mv.y.atan2(mv.x), dt);
    }
    let heading = vec2(angle.cos(), angle.sin());
    let left = vec2(-heading.y, heading.x);
    input.throttle = mv.dot(heading).clamp(-1.0, 1.0);
    input.strafe = mv.dot(left).clamp(-1.0, 1.0);
}

/// Rotate `v` back toward `axis` until it sits within `cone` radians of it,
/// preserving its length. Vectors already inside the cone (and degenerate
/// inputs) pass through unchanged.
pub fn clamp_toward(v: Vec2, axis: Vec2, cone: f32) -> Vec2 {
    let len = v.length();
    if len < 1e-6 || axis.length_squared() < 1e-6 {
        return v;
    }
    let base = axis.y.atan2(axis.x);
    let err = wrap_pi(v.y.atan2(v.x) - base);
    if err.abs() <= cone {
        return v;
    }
    let a = base + err.clamp(-cone, cone);
    vec2(a.cos(), a.sin()) * len
}

fn normalize_to(v: Vec2, len: f32) -> Vec2 {
    let l = v.length();
    if l < 1e-6 {
        Vec2::ZERO
    } else {
        v * (len / l)
    }
}

fn wrap_pi(a: f32) -> f32 {
    let mut a = a % TAU;
    if a > PI {
        a -= TAU;
    } else if a < -PI {
        a += TAU;
    }
    a
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Chunk, Rock, World};

    fn empty_world() -> World {
        World::new(1)
    }

    // Farthest a bullet can physically travel before its life runs out (tiles).
    const BULLET_REACH: f32 = Bullet::SPEED * Bullet::LIFE;

    fn view(pos: Vec2, vel: Vec2, angle: f32) -> PlayerView {
        PlayerView {
            pos,
            vel,
            angle,
            turret_angle: angle,
            radius: 0.65,
            hp: Player::MAX_HULL + Player::MAX_SHIELD,
            dead: false,
        }
    }

    // ── Intercept ──────────────────────────────────────────────────────

    #[test]
    fn intercept_stationary_target() {
        // Target 10 tiles to the +x, both still. Aim straight at it, t = d/speed.
        let (dir, t) =
            solve_intercept(vec2(10.0, 0.0), Vec2::ZERO, Bullet::SPEED, Bullet::LIFE).unwrap();
        assert!((dir - vec2(1.0, 0.0)).length() < 1e-3);
        assert!((t - 10.0 / Bullet::SPEED).abs() < 1e-3);
    }

    #[test]
    fn intercept_lateral_leads_ahead() {
        // Target ahead moving +y: aim must lead in +y.
        let (dir, _) =
            solve_intercept(vec2(10.0, 0.0), vec2(0.0, 5.0), Bullet::SPEED, Bullet::LIFE).unwrap();
        assert!(dir.y > 0.0, "should lead the target: {dir:?}");
        assert!(dir.x > 0.0);
    }

    #[test]
    fn intercept_approaching_target() {
        // Head-on closing target: still a valid, sooner intercept straight ahead.
        let (dir, t) = solve_intercept(
            vec2(10.0, 0.0),
            vec2(-4.0, 0.0),
            Bullet::SPEED,
            Bullet::LIFE,
        )
        .unwrap();
        assert!((dir - vec2(1.0, 0.0)).length() < 1e-3);
        assert!(t < 10.0 / Bullet::SPEED, "closing target is hit sooner");
    }

    #[test]
    fn intercept_unreachable_fast_target() {
        // Target fleeing along +x faster than the bullet closes — no intercept.
        let res = solve_intercept(
            vec2(5.0, 0.0),
            vec2(100.0, 0.0),
            Bullet::SPEED,
            Bullet::LIFE,
        );
        assert!(res.is_none());
    }

    #[test]
    fn intercept_expired_lifetime() {
        // Reachable in principle but only far beyond the bullet lifetime.
        let far = Bullet::SPEED * Bullet::LIFE * 4.0;
        let res = solve_intercept(vec2(far, 0.0), Vec2::ZERO, Bullet::SPEED, Bullet::LIFE);
        assert!(res.is_none());
    }

    // ── Line-of-fire ───────────────────────────────────────────────────

    #[test]
    fn line_of_fire_blocked_and_clear() {
        let rocks = [(vec2(5.0, 0.0), 0.5)];
        // Straight through the rock — blocked.
        assert!(!segment_clear(
            vec2(0.0, 0.0),
            vec2(10.0, 0.0),
            Bullet::RADIUS,
            rocks
        ));
        // Parallel offset that clears the expanded rock.
        assert!(segment_clear(
            vec2(0.0, 3.0),
            vec2(10.0, 3.0),
            Bullet::RADIUS,
            rocks
        ));
    }

    #[test]
    fn line_of_fire_expands_by_radius() {
        // Rock just grazing the expanded corridor is a block.
        let r = 0.5;
        let rocks = [(vec2(5.0, r + Bullet::RADIUS - 0.01), r)];
        assert!(!segment_clear(
            vec2(0.0, 0.0),
            vec2(10.0, 0.0),
            Bullet::RADIUS,
            rocks
        ));
        let rocks_far = [(vec2(5.0, r + Bullet::RADIUS + 0.2), r)];
        assert!(segment_clear(
            vec2(0.0, 0.0),
            vec2(10.0, 0.0),
            Bullet::RADIUS,
            rocks_far
        ));
    }

    // ── Avoidance steering ─────────────────────────────────────────────

    #[test]
    fn avoid_clear_path_is_zero() {
        let (steer, urgent) = avoidance_steer(Vec2::ZERO, 0.65, vec2(1.0, 0.0), 5.0, &[]);
        assert_eq!(steer, Vec2::ZERO);
        assert!(!urgent);
    }

    #[test]
    fn avoid_rock_offset_plus_y_steers_away_minus_y() {
        // Rock ahead offset to +y → steer must push toward −y (away from it).
        let rocks = [(vec2(2.0, 0.3), 0.5)];
        let (steer, _) = avoidance_steer(Vec2::ZERO, 0.65, vec2(1.0, 0.0), 5.0, &rocks);
        assert!(steer.y < 0.0, "should steer away (−y): {steer:?}");
    }

    #[test]
    fn avoid_rock_offset_minus_y_steers_away_plus_y() {
        let rocks = [(vec2(2.0, -0.3), 0.5)];
        let (steer, _) = avoidance_steer(Vec2::ZERO, 0.65, vec2(1.0, 0.0), 5.0, &rocks);
        assert!(steer.y > 0.0, "should steer away (+y): {steer:?}");
    }

    #[test]
    fn avoid_head_on_steers_to_a_side() {
        let rocks = [(vec2(1.5, 0.0), 0.5)];
        let (steer, urgent) = avoidance_steer(Vec2::ZERO, 0.65, vec2(1.0, 0.0), 5.0, &rocks);
        assert!(
            steer.y.abs() > 1e-3,
            "head-on must pick a lateral side: {steer:?}"
        );
        assert!(urgent, "close head-on rock is urgent");
    }

    #[test]
    fn avoid_overlapping_rock_pushes_out_and_is_urgent() {
        let rocks = [(vec2(0.2, 0.0), 0.5)];
        let (steer, urgent) = avoidance_steer(Vec2::ZERO, 0.65, vec2(1.0, 0.0), 5.0, &rocks);
        assert!(urgent);
        assert!(
            steer.x < 0.0,
            "should push back away from overlapping rock: {steer:?}"
        );
    }

    // ── Behavior selection ─────────────────────────────────────────────

    #[test]
    fn behavior_inactive_when_self_dead() {
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 7);
        let mut me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        me.dead = true;
        let opp = view(vec2(5.0, 0.0), Vec2::ZERO, 0.0);
        let inp = a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert_eq!(a.debug.behavior, Behavior::Inactive);
        assert!(!inp.fire && !inp.fire_rocket);
        assert_eq!(inp.throttle, 0.0);
    }

    #[test]
    fn behavior_loiter_when_opponent_dead() {
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 7);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let mut opp = view(vec2(5.0, 0.0), Vec2::ZERO, 0.0);
        opp.dead = true;
        a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert_eq!(a.debug.behavior, Behavior::Loiter);
    }

    #[test]
    fn behavior_pursue_when_far() {
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 7);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(30.0, 0.0), Vec2::ZERO, 0.0); // far outside the band
        a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert_eq!(a.debug.behavior, Behavior::Pursue);
    }

    #[test]
    fn behavior_orbit_inside_band() {
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 7);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        // Distance ~ preferred (5.5) → orbit.
        let opp = view(vec2(5.5, 0.0), Vec2::ZERO, 0.0);
        a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert_eq!(a.debug.behavior, Behavior::Orbit);
    }

    #[test]
    fn behavior_avoid_on_imminent_obstacle() {
        // Put a rock directly between the AI and the opponent, right ahead.
        let mut world = World::new(999);
        world.chunks.clear();
        world.chunks.insert(
            (0, 0),
            Chunk {
                rocks: vec![Rock {
                    pos: vec2(1.5, 0.0),
                    radius: 0.6,
                    variant: 0,
                    hp: 100.0,
                }],
            },
        );
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 3);
        // Moving forward toward the rock.
        let me = view(Vec2::ZERO, vec2(4.0, 0.0), 0.0);
        let opp = view(vec2(8.0, 0.0), Vec2::ZERO, 0.0);
        a.tick(me, opp, &world, 1.0 / 60.0);
        assert_eq!(a.debug.behavior, Behavior::Avoid);
        assert!(a.debug.avoid_urgent);
    }

    #[test]
    fn gun_fires_when_aligned_and_clear() {
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 1);
        // Facing +x, opponent dead ahead, stationary, in range.
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(6.0, 0.0), Vec2::ZERO, 0.0);
        // Hard has tiny aim error; a few ticks let it settle onto the target.
        let mut fired = false;
        for _ in 0..20 {
            let inp = a.tick(me, opp, &empty_world(), 1.0 / 60.0);
            fired |= inp.fire;
        }
        assert!(
            fired,
            "aligned Hard AI should fire at a clear, in-range target"
        );
    }

    #[test]
    fn gun_holds_fire_through_rock() {
        // Rock right in the line of fire → never shoot into it.
        let mut world = World::new(1);
        world.chunks.clear();
        world.chunks.insert(
            (0, 0),
            Chunk {
                rocks: vec![Rock {
                    pos: vec2(4.0, 0.0),
                    radius: 0.6,
                    variant: 0,
                    hp: 100.0,
                }],
            },
        );
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 1);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(6.0, 0.0), Vec2::ZERO, 0.0);
        // Fewer ticks than Hard's press_cover_delay (0.1s @ 60fps = 6 ticks),
        // so this only covers the "not yet pinned long enough" window — see
        // `ai_presses_cover_after_sustained_block_not_momentary` below for the
        // sustained-block case, where firing on the rock itself is expected.
        let mut fired = false;
        for _ in 0..3 {
            let inp = a.tick(me, opp, &world, 1.0 / 60.0);
            fired |= inp.fire;
        }
        assert!(!fired, "must not fire bullets through a blocking rock");
    }

    #[test]
    fn segment_blocker_reports_the_blocking_rock() {
        let rocks = [(vec2(5.0, 0.0), 0.5)];
        // Straight through the rock — blocked, reports the rock.
        assert_eq!(
            segment_blocker(vec2(0.0, 0.0), vec2(10.0, 0.0), Bullet::RADIUS, rocks),
            Some((vec2(5.0, 0.0), 0.5))
        );
        // Parallel offset that clears the expanded rock — no blocker.
        assert_eq!(
            segment_blocker(vec2(0.0, 3.0), vec2(10.0, 3.0), Bullet::RADIUS, rocks),
            None
        );
    }

    #[test]
    fn ai_presses_cover_only_after_sustained_block_not_momentary() {
        // Rock directly on the line between the AI and its target, so the
        // gunfire leaf is blocked every tick — the classic "opponent pinned
        // behind cover" case.
        let mut world = World::new(1);
        world.chunks.clear();
        world.chunks.insert(
            (0, 0),
            Chunk {
                rocks: vec![Rock {
                    pos: vec2(4.0, 0.0),
                    radius: 0.6,
                    variant: 0,
                    hp: 100.0,
                }],
            },
        );
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 1);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(6.0, 0.0), Vec2::ZERO, 0.0);
        let dt = 1.0 / 60.0;

        // Well under Hard's press_cover_delay (0.1s): a momentary occlusion
        // must not trigger rock-pressing.
        let mut fired_early = false;
        for _ in 0..3 {
            fired_early |= a.tick(me, opp, &world, dt).fire;
        }
        assert!(!fired_early, "must not press cover on a brief occlusion");

        // Kept pinned well past the threshold: the AI should give up waiting
        // for a clear shot and fire on the blocking rock instead.
        let mut fired_late = false;
        for _ in 0..120 {
            fired_late |= a.tick(me, opp, &world, dt).fire;
        }
        assert!(
            fired_late,
            "should press cover (fire on the blocking rock) once genuinely pinned"
        );
    }

    #[test]
    fn ai_does_not_press_cover_when_a_clear_shot_exists() {
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 1);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(6.0, 0.0), Vec2::ZERO, 0.0);
        let dt = 1.0 / 60.0;
        // No rocks anywhere — a clear shot always exists, so the press timer
        // must never build up regardless of how long the AI runs.
        for _ in 0..300 {
            a.tick(me, opp, &empty_world(), dt);
        }
        assert_eq!(
            a.rock_press_timer, 0.0,
            "clear line of fire must keep the rock-press timer at zero"
        );
    }

    #[test]
    fn rocket_pulse_is_eligible_and_paced() {
        // Hard AI, opponent in the rocket window, aligned, clear path.
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 5);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(10.0, 0.0), Vec2::ZERO, 0.0);
        // Prime the timer so the first attempt can fire.
        a.rocket_timer = 0.0;
        let mut pulses = 0;
        let mut prev_pulse = false;
        for _ in 0..400 {
            let inp = a.tick(me, opp, &empty_world(), 1.0 / 60.0);
            if inp.fire_rocket {
                pulses += 1;
                assert!(
                    !prev_pulse,
                    "rocket pulses must be paced, never two frames in a row"
                );
            }
            prev_pulse = inp.fire_rocket;
        }
        // Over ~6.6 s the difficulty-scaled opportunity rolls produce launches,
        // but pacing keeps them far below one-per-frame.
        assert!(pulses >= 1, "eligible Hard AI should launch a rocket");
        assert!(pulses < 400, "rocket pacing must throttle launches");
    }

    #[test]
    fn rocket_out_of_range_never_fires() {
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 5);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(2.0, 0.0), Vec2::ZERO, 0.0); // inside ROCKET_MIN
        a.rocket_timer = 0.0;
        let mut any = false;
        for _ in 0..30 {
            any |= a.tick(me, opp, &empty_world(), 1.0 / 60.0).fire_rocket;
        }
        assert!(!any, "point-blank target is below the rocket window");
    }

    #[test]
    fn firing_ranges_are_within_bullet_reach() {
        // The AI must never open fire (or expect a threat) beyond where a bullet
        // can physically travel, or shots fall short. Guards future bullet tuning.
        const {
            assert!(
                GUN_RANGE <= BULLET_REACH,
                "gun range must not exceed bullet reach"
            )
        };
        const {
            assert!(
                THREAT_RANGE <= BULLET_REACH,
                "threat range must not exceed bullet reach"
            )
        };
        // And every difficulty's outer combat edge must sit inside gun range.
        for d in [AiDifficulty::Easy, AiDifficulty::Normal, AiDifficulty::Hard] {
            let t = Tuning::for_difficulty(d);
            assert!(
                t.preferred + t.band <= GUN_RANGE,
                "{d:?} orbits past gun range"
            );
        }
    }

    // ── Aim noise: bounded, deterministic, cadence-gated ────────────────

    #[test]
    fn aim_error_is_bounded() {
        let mut a = AiAgent::new(0, AiDifficulty::Easy, 42);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let dist = 9.0;
        let opp = view(vec2(dist, 0.0), Vec2::ZERO, 0.0);
        // Bound scales with range via range_scale (super-linear off-screen).
        let bound = Tuning::for_difficulty(AiDifficulty::Easy).aim_error * range_scale(dist);
        for _ in 0..200 {
            a.tick(me, opp, &empty_world(), 1.0 / 30.0);
            assert!(a.aim_error.length() <= bound + 1e-4);
        }
    }

    #[test]
    fn aim_noise_grows_with_range() {
        // Same seed, same reaction cadence: the far sample must be able to
        // produce a larger error than the base (close) bound allows.
        let base = Tuning::for_difficulty(AiDifficulty::Normal).aim_error;
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let mut max_far = 0.0_f32;
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 7);
        let opp = view(vec2(18.0, 0.0), Vec2::ZERO, 0.0);
        for _ in 0..400 {
            a.tick(me, opp, &empty_world(), 1.0 / 30.0);
            max_far = max_far.max(a.aim_error.length());
        }
        assert!(
            max_far > base,
            "long-range aim error ({max_far}) should exceed the close-range base ({base})"
        );
    }

    #[test]
    fn kiting_opponent_triggers_aggressive_close() {
        // Opponent far, facing us (angle π ⇒ pointing back toward the origin),
        // and backing away (+x velocity) — the classic backpedal-and-shoot.
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 3);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(8.0, 0.0), vec2(5.0, 0.0), PI);
        a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert!(a.kiting, "should recognise the kite");
        assert_eq!(a.debug.behavior, Behavior::Pursue);
    }

    #[test]
    fn slow_drift_away_from_a_stalled_ai_is_a_kite() {
        // Opponent facing us and easing back at 0.5 tiles/s — under RECEDE_SPEED,
        // so the absolute test alone misses it — while we sit at the preferred
        // distance with no velocity. The gap is opening; that is a kite.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 5);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(t.preferred, 0.0), vec2(0.5, 0.0), PI);
        a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert!(a.kiting, "an opening gap should read as a kite");
    }

    #[test]
    fn kite_flag_is_held_briefly_when_the_signature_drops() {
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 5);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let dt = 1.0 / 60.0;
        a.tick(
            me,
            view(vec2(8.0, 0.0), vec2(5.0, 0.0), PI),
            &empty_world(),
            dt,
        );
        assert!(a.kiting);
        // Opponent turns away (out of the threat cone): the flag holds, then drops.
        let broken = view(vec2(8.0, 0.0), vec2(5.0, 0.0), 0.0);
        a.tick(me, broken, &empty_world(), dt);
        assert!(
            a.kiting,
            "flag should survive a momentary break in the signature"
        );
        for _ in 0..(KITE_HOLD / dt) as i32 + 2 {
            a.tick(me, broken, &empty_world(), dt);
        }
        assert!(!a.kiting, "flag should expire after KITE_HOLD");
    }

    #[test]
    fn holding_the_band_does_not_stall_the_ship() {
        // Parked at exactly the preferred distance from a stationary opponent:
        // the distance error is zero, so only the weave keeps the ship moving.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 9);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(t.preferred, 0.0), Vec2::ZERO, PI);
        let inp = a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert_eq!(a.debug.behavior, Behavior::Orbit);
        assert!(
            inp.throttle.abs() > 0.2,
            "band hold must keep the ship moving, got throttle {}",
            inp.throttle
        );
    }

    #[test]
    fn band_hold_keeps_pace_with_a_backing_opponent() {
        // Just inside the band's outer edge with the opponent backing off at
        // 6 tiles/s: the feed-forward term must ask for real closing throttle,
        // not the small correction the distance error alone would give.
        // Differential against a stationary opponent on the same seed, so the
        // (random-signed) weave cancels and only the feed-forward term is left.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let dt = 1.0 / 60.0;
        // Facing away, so this is not a kite — the band controller must cope.
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 13);
        let chasing = a.tick(
            me,
            view(vec2(t.preferred, 0.0), vec2(6.0, 0.0), 0.0),
            &empty_world(),
            dt,
        );
        let mut b = AiAgent::new(0, AiDifficulty::Normal, 13);
        let holding = b.tick(
            me,
            view(vec2(t.preferred, 0.0), Vec2::ZERO, 0.0),
            &empty_world(),
            dt,
        );
        assert!(!a.kiting, "an opponent facing away is not kiting us");
        assert_eq!(a.debug.behavior, Behavior::Orbit);
        // 6 tiles/s of chase needs ~0.4 throttle in steady state.
        assert!(
            chasing.throttle - holding.throttle > 0.35,
            "should match the opponent's radial speed: {} vs {}",
            chasing.throttle,
            holding.throttle
        );
    }

    /// What a closed-loop run reports back.
    pub struct Sim {
        pub mean_speed: f32,
        pub closest: f32,
        pub farthest: f32,
        pub final_dist: f32,
        /// Largest angle between the hull nose and the bearing to the opponent
        /// (radians), sampled only after the first second so the initial
        /// swing-on doesn't dominate.
        pub max_bearing: f32,
        /// Fraction of frames the gun leaf asked to fire.
        pub fire_frac: f32,
        /// Mean speed over the final two seconds — near zero means the agent ended
        /// the run parked (stuck on a rock, starved of throttle, …).
        pub late_speed: f32,
    }

    /// Setup for a closed-loop run. `Default` is a Normal agent against an
    /// opponent parked at its preferred range facing it, in an empty world.
    #[derive(Clone)]
    pub struct Run {
        pub difficulty: AiDifficulty,
        pub seed: u64,
        pub opp_start: f32,
        pub opp_vel: Vec2,
        pub opp_angle: f32,
        pub secs: f32,
        /// Land a hit on the AI at this time (seconds), to exercise withdrawal.
        pub hurt_at: Option<f32>,
        /// Damage dealt at `hurt_at`. Defaults to a token hit; tests that need
        /// to actually cross `Tuning::withdraw_hp_frac` should size this
        /// explicitly against the difficulty under test.
        pub hurt_amount: f32,
        pub rocks: Vec<(Vec2, f32)>,
    }

    impl Default for Run {
        fn default() -> Self {
            Self {
                difficulty: AiDifficulty::Normal,
                seed: 17,
                opp_start: Tuning::for_difficulty(AiDifficulty::Normal).preferred,
                opp_vel: Vec2::ZERO,
                opp_angle: PI,
                secs: 6.0,
                hurt_at: None,
                hurt_amount: 30.0,
                rocks: Vec::new(),
            }
        }
    }

    /// Closed-loop sim: drive a real `Player` from the agent's input against a
    /// scripted opponent.
    pub fn simulate(run: Run) -> Sim {
        let mut world = World::new(1);
        world.chunks.clear();
        world.chunks.insert(
            (0, 0),
            Chunk {
                rocks: run
                    .rocks
                    .iter()
                    .map(|&(pos, radius)| Rock {
                        pos,
                        radius,
                        variant: 0,
                        hp: 100.0,
                    })
                    .collect(),
            },
        );
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, run.difficulty, run.seed);
        let mut me = Player::new(Vec2::ZERO);
        let mut opp = Player::new(vec2(run.opp_start, 0.0));
        opp.vel = run.opp_vel;
        opp.angle = run.opp_angle;
        let mut speed_sum = 0.0;
        let mut late_sum = 0.0;
        let mut late_steps = 0;
        let mut closest = f32::MAX;
        let mut farthest = 0.0_f32;
        let mut max_bearing = 0.0_f32;
        let mut fire_frames = 0;
        let steps = (run.secs / dt) as i32;
        for step in 0..steps {
            let t = step as f32 * dt;
            if let Some(h) = run.hurt_at {
                if t >= h && t < h + dt {
                    me.apply_damage(run.hurt_amount);
                }
            }
            let input = a.tick(PlayerView::of(&me), PlayerView::of(&opp), &world, dt);
            if input.fire {
                fire_frames += 1;
            }
            me.update(&input, dt);
            // Same order as `GameState::update`: rocks push the hull back out
            // and kill the inward velocity, so the sim reproduces scraping
            // along a rock face rather than sliding through it.
            crate::world::resolve_vehicle_rocks(&mut me, &world);
            opp.pos += opp.vel * dt; // scripted: no damping, holds its speed
            speed_sum += me.vel.length();
            if t > run.secs - 2.0 {
                late_sum += me.vel.length();
                late_steps += 1;
            }
            let to_opp = opp.pos - me.pos;
            closest = closest.min(to_opp.length());
            farthest = farthest.max(to_opp.length());
            if t > 1.0 {
                max_bearing = max_bearing.max(wrap_pi(to_opp.y.atan2(to_opp.x) - me.angle).abs());
            }
        }
        Sim {
            mean_speed: speed_sum / steps as f32,
            closest,
            farthest,
            final_dist: (opp.pos - me.pos).length(),
            max_bearing,
            fire_frac: fire_frames as f32 / steps as f32,
            late_speed: late_sum / late_steps.max(1) as f32,
        }
    }

    #[test]
    fn ai_does_not_park_against_a_standing_opponent() {
        // The reported symptom: the AI coasts to a halt at the preferred range
        // and sits there. It should keep shuffling in and out of the band.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let s = simulate(Run {
            secs: 4.0,
            ..Run::default()
        });
        assert!(
            s.mean_speed > 1.0,
            "AI stalled at range: mean speed {}",
            s.mean_speed
        );
        assert!(
            (s.final_dist - t.preferred).abs() < t.band * 2.0,
            "AI should still hold the band, ended at {}",
            s.final_dist
        );
    }

    #[test]
    fn ai_keeps_pace_with_a_slow_backpedal() {
        // Retreat below RECEDE_SPEED — the case the old absolute-velocity test
        // missed entirely. Over 5 s the opponent covers 3 tiles; the AI must
        // stay in contact rather than letting the gap ratchet open.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let s = simulate(Run {
            seed: 23,
            opp_vel: vec2(0.6, 0.0),
            secs: 5.0,
            ..Run::default()
        });
        assert!(
            s.final_dist < t.preferred + t.band,
            "AI lost ground to a slow backpedal, ended at {}",
            s.final_dist
        );
        assert!(
            s.closest > 1.5,
            "closing pressure should not ram the opponent ({})",
            s.closest
        );
    }

    #[test]
    fn kite_pursuit_breaks_off_at_the_inner_ring() {
        // The kite flag is held (KITE_HOLD) past the moment the signature drops,
        // so the sprint must not outrank the inner-ring break-off — otherwise the
        // AI charges a retreating opponent all the way into a collision.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let s = simulate(Run {
            seed: 31,
            opp_vel: vec2(4.0, 0.0),
            ..Run::default()
        });
        // Normal presses, so the break-off ring is the tighter contact one — but
        // two hulls touch at ~1.3 tiles, and the sprint must still stop short.
        assert!(
            s.closest > t.inner * PRESS_INNER_FRAC - 0.5,
            "kite sprint overran the break-off ring ({})",
            s.closest
        );
    }

    #[test]
    fn advancing_opponent_is_not_kiting() {
        // Opponent facing us but charging in (−x velocity) — not a kite.
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 3);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(8.0, 0.0), vec2(-5.0, 0.0), PI);
        a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert!(!a.kiting, "an approaching opponent is not kiting");
    }

    #[test]
    fn aim_error_updates_only_at_reaction_interval() {
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 11);
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(9.0, 0.0), Vec2::ZERO, 0.0);
        let dt = 1.0 / 120.0;
        a.tick(me, opp, &empty_world(), dt);
        let first = a.aim_error;
        // Well within the 0.22 s reaction window — no resample.
        for _ in 0..5 {
            a.tick(me, opp, &empty_world(), dt);
            assert_eq!(a.aim_error, first, "aim error must hold between samples");
        }
    }

    #[test]
    fn deterministic_for_fixed_seed() {
        let run = || {
            let mut a = AiAgent::new(1, AiDifficulty::Normal, 2024);
            let me = view(Vec2::ZERO, Vec2::ZERO, 0.3);
            let opp = view(vec2(7.0, 2.0), vec2(1.0, 0.0), 0.0);
            let mut acc = Vec::new();
            for _ in 0..60 {
                let inp = a.tick(me, opp, &empty_world(), 1.0 / 60.0);
                acc.push((inp.throttle, inp.turn, inp.fire, inp.fire_rocket));
            }
            acc
        };
        assert_eq!(run(), run(), "same seed ⇒ identical control stream");
    }

    #[test]
    fn agent_order_independent() {
        // Two AI agents deciding from the same snapshots — order must not matter.
        let world = empty_world();
        let v0 = view(Vec2::ZERO, vec2(1.0, 0.0), 0.0);
        let v1 = view(vec2(9.0, 1.0), vec2(-1.0, 0.5), PI);
        let dt = 1.0 / 60.0;

        let mut a0 = AiAgent::new(0, AiDifficulty::Normal, 100);
        let mut a1 = AiAgent::new(1, AiDifficulty::Hard, 200);
        let r0_first = a0.tick(v0, v1, &world, dt);
        let r1_second = a1.tick(v1, v0, &world, dt);

        let mut b0 = AiAgent::new(0, AiDifficulty::Normal, 100);
        let mut b1 = AiAgent::new(1, AiDifficulty::Hard, 200);
        let r1_first = b1.tick(v1, v0, &world, dt);
        let r0_second = b0.tick(v0, v1, &world, dt);

        assert_eq!(r0_first.throttle, r0_second.throttle);
        assert_eq!(r0_first.turn, r0_second.turn);
        assert_eq!(r1_second.throttle, r1_first.throttle);
        assert_eq!(r1_second.turn, r1_first.turn);
    }

    // ── Pressing the attack (Normal / Hard) ────────────────────────────

    #[test]
    fn clamp_toward_folds_into_the_cone_and_keeps_length() {
        let cone = 20.0_f32.to_radians();
        let axis = vec2(1.0, 0.0);
        // Already inside: untouched.
        let inside = vec2(1.0, 0.2);
        assert_eq!(clamp_toward(inside, axis, cone), inside);
        // Outside: folded back to the cone edge, same length, same side.
        let out = vec2(0.0, 3.0); // 90° off, length 3
        let c = clamp_toward(out, axis, cone);
        assert!((c.length() - 3.0).abs() < 1e-4, "length preserved: {c:?}");
        assert!(
            (c.y.atan2(c.x) - cone).abs() < 1e-4,
            "on the +cone edge: {c:?}"
        );
        let c = clamp_toward(vec2(0.0, -3.0), axis, cone);
        assert!(
            (c.y.atan2(c.x) + cone).abs() < 1e-4,
            "on the −cone edge: {c:?}"
        );
        // Degenerate inputs pass through.
        assert_eq!(clamp_toward(Vec2::ZERO, axis, cone), Vec2::ZERO);
        assert_eq!(clamp_toward(inside, Vec2::ZERO, cone), inside);
    }

    #[test]
    fn press_keeps_the_gun_bearing_on_the_target() {
        // The turret traverses only ±TURRET_MAX off the nose, so a stand-off
        // orbit — which points its flank at the opponent — can barely shoot.
        // Pressing holds the nose (and the gun) on the target instead. All
        // three difficulties press now.
        for d in [AiDifficulty::Easy, AiDifficulty::Normal, AiDifficulty::Hard] {
            let t = Tuning::for_difficulty(d);
            let s = simulate(Run {
                difficulty: d,
                opp_start: t.preferred,
                ..Run::default()
            });
            assert!(
                s.max_bearing < PRESS_CONE + 0.15,
                "{d:?} drifted off the bearing: {:.1}°",
                s.max_bearing.to_degrees()
            );
            // Easy's much larger aim_error/reaction lag (see Tuning::for_difficulty)
            // means its computed aim direction strays out of its own fire_cone far
            // more often, so it shoots less even while nose-on. Normal/Hard should
            // still be shooting most of the engagement.
            let min_fire_frac = if d == AiDifficulty::Easy { 0.3 } else { 0.5 };
            assert!(
                s.fire_frac > min_fire_frac,
                "{d:?} should be shooting a meaningful fraction of the engagement, got {:.2}",
                s.fire_frac
            );
        }
    }

    #[test]
    fn press_gives_no_ground_until_it_is_hurt() {
        for d in [AiDifficulty::Easy, AiDifficulty::Normal, AiDifficulty::Hard] {
            let t = Tuning::for_difficulty(d);
            let engage = Run {
                difficulty: d,
                opp_start: t.preferred,
                ..Run::default()
            };
            let calm = simulate(engage.clone());
            assert!(
                calm.farthest <= t.preferred + 0.2,
                "{d:?} withdrew without being hit, reached {:.2}",
                calm.farthest
            );
            assert!(
                calm.closest < t.preferred - 1.5,
                "{d:?} should press in past the stand-off range, closest {:.2}",
                calm.closest
            );
            // A hit that brings the pool down to the withdraw threshold buys
            // the retreat that geometry alone does not.
            let max_hp = Player::MAX_HULL + Player::MAX_SHIELD;
            let hurt_amount = (1.0 - t.withdraw_hp_frac) * max_hp + 10.0;
            let hurt = simulate(Run {
                hurt_at: Some(2.0),
                hurt_amount,
                ..engage
            });
            assert!(
                hurt.farthest > t.preferred + t.band,
                "{d:?} should give ground once hurt, reached {:.2}",
                hurt.farthest
            );
        }
    }

    #[test]
    fn press_opens_the_range_in_reverse_not_by_turning_tail() {
        // Falling back to `preferred` must not hand the opponent our tail: the
        // hull stays nose-on and the range opens on the (capped) reverse.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let world = empty_world();
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 9);
        a.band_sign = 1.0; // outbound leg
        let mut me = Player::new(Vec2::ZERO);
        let opp = Player::new(vec2(t.preferred - 1.5, 0.0));
        let start = (opp.pos - me.pos).length();
        let mut max_bearing = 0.0_f32;
        for _ in 0..60 {
            // Stop at the outbound setpoint: the shuttle flips there and the
            // next inbound pass begins.
            if a.band_sign < 0.0 {
                break;
            }
            let input = a.tick(PlayerView::of(&me), PlayerView::of(&opp), &world, dt);
            assert!(
                input.throttle <= 0.0,
                "should back out, got {}",
                input.throttle
            );
            me.update(&input, dt);
            let to_opp = opp.pos - me.pos;
            max_bearing = max_bearing.max(wrap_pi(to_opp.y.atan2(to_opp.x) - me.angle).abs());
        }
        assert!(
            (opp.pos - me.pos).length() > start,
            "the range should actually open"
        );
        assert!(
            max_bearing < PRESS_CONE + 0.15,
            "never turn tail while backing off: {:.1}°",
            max_bearing.to_degrees()
        );
    }

    #[test]
    fn pivot_coasts_onto_the_bearing_instead_of_powering_through() {
        // Sprinting +x with the opponent behind us: turn rate falls off with
        // speed, so the pressing agent cuts throttle and lets the hull swing.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 3);
        let me = view(Vec2::ZERO, vec2(Player::MAX_SPEED, 0.0), 0.0);
        let opp = view(vec2(-t.preferred, 0.0), Vec2::ZERO, 0.0);
        let inp = a.tick(me, opp, &empty_world(), 1.0 / 60.0);
        assert_eq!(a.debug.behavior, Behavior::Pivot);
        assert_eq!(inp.throttle, 0.0, "coast, don't power through the turn");
        assert!(inp.turn.abs() > 0.5, "and turn hard: {}", inp.turn);
    }

    #[test]
    fn a_rock_in_the_way_never_parks_the_ship() {
        // Cover between the two ships. Pressing pins the nose on the opponent,
        // so the rock sits squarely on the path — the agent has to route around
        // it and keep fighting, not stall against it.
        for d in [AiDifficulty::Easy, AiDifficulty::Normal, AiDifficulty::Hard] {
            for rock in [vec2(3.0, 0.0), vec2(2.0, 0.6), vec2(4.0, -0.4)] {
                for seed in [17u64, 23, 31] {
                    let s = simulate(Run {
                        difficulty: d,
                        seed,
                        secs: 10.0,
                        rocks: vec![(rock, 0.9)],
                        ..Run::default()
                    });
                    assert!(
                        s.late_speed > 0.5,
                        "{d:?} seed {seed} parked on the rock at {rock:?}: \
                         late speed {:.2}, mean {:.2}",
                        s.late_speed,
                        s.mean_speed
                    );
                    assert!(
                        s.closest > 1.0,
                        "{d:?} seed {seed} rammed the opponent: {:.2}",
                        s.closest
                    );
                }
            }
        }
    }

    #[test]
    fn a_wedged_ship_backs_out_and_frees_itself() {
        // A pocket the reactive steer cannot solve: rocks either side of the
        // line to the opponent, close enough that the ship grinds between them.
        // The outcome test (thrust on, no motion) has to notice and reverse.
        let world = {
            let mut w = World::new(1);
            w.chunks.clear();
            w.chunks.insert(
                (0, 0),
                Chunk {
                    rocks: vec![
                        Rock {
                            pos: vec2(2.2, 1.3),
                            radius: 1.2,
                            variant: 0,
                            hp: 100.0,
                        },
                        Rock {
                            pos: vec2(2.2, -1.3),
                            radius: 1.2,
                            variant: 0,
                            hp: 100.0,
                        },
                    ],
                },
            );
            w
        };
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, AiDifficulty::Hard, 17);
        let mut me = Player::new(vec2(1.4, 0.0)); // jammed in the gap
        let mut opp = Player::new(vec2(7.0, 0.0));
        opp.angle = PI;
        let mut freed = false;
        for _ in 0..300 {
            let input = a.tick(PlayerView::of(&me), PlayerView::of(&opp), &world, dt);
            me.update(&input, dt);
            crate::world::resolve_vehicle_rocks(&mut me, &world);
            // Clear of the gap once it is back out past both rock centres.
            freed |= me.pos.x < 0.5 || (me.pos.x > 3.5 && me.pos.y.abs() < 1.0);
        }
        assert!(
            freed,
            "ship never escaped the pocket, ended at {:?}",
            me.pos
        );
    }

    #[test]
    fn unstick_only_fires_on_thrust_without_motion() {
        let world = empty_world();
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 5);
        // Pinned: full throttle asked for, nothing moving. Feed the agent the
        // same stalled snapshot every frame.
        let me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(12.0, 0.0), Vec2::ZERO, PI); // far → pursue, throttle up
        let mut backed_out = false;
        for _ in 0..((STUCK_TIME + 0.2) / dt) as i32 {
            backed_out |= a.tick(me, opp, &world, dt).throttle < 0.0;
        }
        assert!(backed_out, "a ship thrusting and not moving must back out");
        // A ship that is simply coasting through a slow moment is not wedged:
        // the throttle stays where the movement branch put it.
        let mut b = AiAgent::new(0, AiDifficulty::Normal, 5);
        let moving = view(Vec2::ZERO, vec2(4.0, 0.0), 0.0);
        for _ in 0..((STUCK_TIME + 0.2) / dt) as i32 {
            assert!(b.tick(moving, opp, &world, dt).throttle > 0.0);
        }
    }

    #[test]
    fn withdraw_expires_and_the_press_resumes() {
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let world = empty_world();
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 5);
        let mut me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(t.preferred, 0.0), Vec2::ZERO, PI);
        a.tick(me, opp, &world, dt); // baseline hp
        assert_ne!(a.debug.behavior, Behavior::Withdraw);
        me.hp = t.withdraw_hp_frac * (Player::MAX_HULL + Player::MAX_SHIELD) - 1.0;
        a.tick(me, opp, &world, dt);
        assert_eq!(
            a.debug.behavior,
            Behavior::Withdraw,
            "a hit that brings the pool to the threshold buys a retreat"
        );
        for _ in 0..(t.withdraw / dt) as i32 + 2 {
            a.tick(me, opp, &world, dt);
        }
        assert_ne!(
            a.debug.behavior,
            Behavior::Withdraw,
            "and the retreat is time-boxed"
        );
    }

    #[test]
    fn normal_presses_through_early_damage() {
        // Normal should stay aggressive after a hit that leaves it well above
        // `withdraw_hp_frac` of its pool, and only give ground once a hit
        // actually brings it down to that threshold.
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let world = empty_world();
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 5);
        let max_hp = Player::MAX_HULL + Player::MAX_SHIELD;
        let mut me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(t.preferred, 0.0), Vec2::ZERO, PI);
        a.tick(me, opp, &world, dt); // baseline hp
        me.hp = max_hp * 0.6; // a real hit, but well above the withdraw threshold
        a.tick(me, opp, &world, dt);
        assert_ne!(
            a.debug.behavior,
            Behavior::Withdraw,
            "should keep pressing while still above the withdraw threshold"
        );
        me.hp = max_hp * t.withdraw_hp_frac - 1.0; // now genuinely low
        a.tick(me, opp, &world, dt);
        assert_eq!(
            a.debug.behavior,
            Behavior::Withdraw,
            "should withdraw once the pool drops to the threshold"
        );
    }

    #[test]
    fn recharge_and_respawn_are_not_damage() {
        let t = Tuning::for_difficulty(AiDifficulty::Normal);
        let world = empty_world();
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, AiDifficulty::Normal, 5);
        let opp = view(vec2(t.preferred, 0.0), Vec2::ZERO, PI);
        let mut me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        me.hp = 100.0;
        a.tick(me, opp, &world, dt);
        // Shield ticking back up must not read as a hit.
        me.hp = 125.0;
        a.tick(me, opp, &world, dt);
        assert_eq!(a.withdraw_timer, 0.0, "regen is not damage");
        // Nor may the respawn refill after a death.
        me.hp = 40.0;
        a.tick(me, opp, &world, dt);
        me.dead = true;
        me.hp = 0.0;
        a.tick(me, opp, &world, dt);
        me.dead = false;
        me.hp = Player::MAX_HULL + Player::MAX_SHIELD;
        a.tick(me, opp, &world, dt);
        assert_eq!(a.withdraw_timer, 0.0, "respawn is not damage");
    }

    #[test]
    fn easy_presses_through_early_damage() {
        // Easy now presses like Normal/Hard, just with a much higher
        // withdraw threshold: it backs off sooner (at 50% of its pool)
        // instead of breaking off attack the instant it takes any damage.
        let t = Tuning::for_difficulty(AiDifficulty::Easy);
        assert!(t.press);
        let world = empty_world();
        let dt = 1.0 / 60.0;
        let mut a = AiAgent::new(0, AiDifficulty::Easy, 5);
        let max_hp = Player::MAX_HULL + Player::MAX_SHIELD;
        let mut me = view(Vec2::ZERO, Vec2::ZERO, 0.0);
        let opp = view(vec2(t.preferred, 0.0), Vec2::ZERO, PI);
        a.tick(me, opp, &world, dt); // baseline hp
        me.hp = max_hp * 0.8; // a hit, but above Easy's withdraw threshold
        a.tick(me, opp, &world, dt);
        assert_ne!(
            a.debug.behavior,
            Behavior::Withdraw,
            "should keep pressing while still above the withdraw threshold"
        );
        me.hp = max_hp * t.withdraw_hp_frac - 1.0; // now genuinely low
        a.tick(me, opp, &world, dt);
        assert_eq!(
            a.debug.behavior,
            Behavior::Withdraw,
            "should withdraw once the pool drops to the threshold"
        );
    }

    #[test]
    fn player_update_tolerates_a_negative_dt() {
        // miniquad's frame timer is wall-clock, not monotonic — an NTP step
        // can hand us a negative dt. `update_turret`'s traverse-rate clamp is
        // `.clamp(-RATE * dt, RATE * dt)`, which panics (min > max) if dt < 0.
        // The real fix clamps dt at its source in main.rs; this just pins the
        // regression so Player::update stays safe if ever called directly.
        let mut p = Player::new(Vec2::ZERO);
        let input = PlayerInput {
            turret_target: vec2(1.0, 0.0),
            ..Default::default()
        };
        p.update(&input, -1.0 / 24.0);
    }

    #[test]
    fn controller_cycles_through_all_kinds() {
        let mut k = ControllerKind::Human;
        let seq: Vec<_> = (0..5)
            .map(|_| {
                let c = k;
                k = k.next();
                c
            })
            .collect();
        assert_eq!(
            seq,
            vec![
                ControllerKind::Human,
                ControllerKind::Ai(AiDifficulty::Easy),
                ControllerKind::Ai(AiDifficulty::Normal),
                ControllerKind::Ai(AiDifficulty::Hard),
                ControllerKind::Human,
            ]
        );
    }
}

/// Behavioural checks for AI rock-pressing (`node_attack`'s cover-pressing
/// leaf) run against a *moving* ship rather than the static `me` snapshots the
/// unit tests above use: the agent's output is integrated through a real
/// `Player`, so the hull steers, the turret traverses, and every shot passes
/// the real heat gate. This is the headless stand-in for the playtest the
/// change asked for — it can't judge how the behaviour *feels*, but it does
/// pin down that pressing engages when an opponent is genuinely behind cover,
/// stays off when a flank is available, and doesn't chatter.
#[cfg(test)]
mod press_behaviour {
    use super::*;
    use crate::world::{Chunk, Rock, World};

    const FIRE_COOLDOWN: f32 = 0.18;
    const DT: f32 = 1.0 / 60.0;

    #[derive(Default)]
    struct Trace {
        /// Time of the first tick spent aiming at cover rather than the foe.
        first_press: Option<f32>,
        press_ticks: i32,
        /// Contiguous runs of pressing. Many short runs = erratic flip-flopping
        /// between target and cover; a few long ones = deliberate pressure.
        episodes: i32,
        rock_shots: i32,
        opp_shots: i32,
        ticks: i32,
    }

    impl Trace {
        fn press_frac(&self) -> f32 {
            self.press_ticks as f32 / self.ticks as f32
        }
    }

    fn world_with(rocks: Vec<Rock>) -> World {
        let mut w = World::new(1);
        w.chunks.clear();
        w.chunks.insert((0, 0), Chunk { rocks });
        w
    }

    /// A lone boulder with the opponent tucked behind it. A flank exists, so a
    /// good agent should mostly take it.
    fn boulder() -> (World, Vec2) {
        let pos = vec2(4.0, 0.0);
        (
            world_with(vec![Rock {
                pos,
                radius: 2.0,
                variant: 0,
                // Never dies: these tests measure targeting intent, not
                // time-to-destroy, and a rock that crumbles mid-run would
                // end the pin before the slower difficulties react.
                hp: 1.0e9,
            }]),
            pos,
        )
    }

    /// A rock wall the opponent sits behind — no flank available, so shooting
    /// through the cover is the only way to make progress. This is the case
    /// the feature exists for.
    fn wall() -> World {
        world_with(
            (-8..=8)
                .map(|i| Rock {
                    pos: vec2(4.0, i as f32 * 1.2),
                    radius: 0.9,
                    variant: 0,
                    hp: 1.0e9,
                })
                .collect(),
        )
    }

    /// Run `diff` against `world` for `secs` with a stationary opponent at
    /// `opp_pos`, counting shots by what the turret was pointed at.
    fn run(diff: AiDifficulty, world: &World, opp_pos: Vec2, cover_x: f32, secs: f32) -> Trace {
        let mut a = AiAgent::new(0, diff, 1);
        let mut me = Player::new(Vec2::ZERO);
        let opp = Player::new(opp_pos);
        let mut t = Trace::default();
        let mut was_pressing = false;

        t.ticks = (secs / DT) as i32;
        for i in 0..t.ticks {
            let inp = a.tick(PlayerView::of(&me), PlayerView::of(&opp), world, DT);
            // Pressing == the turret is being sent at cover, not at the foe.
            // Cover always sits at x == cover_x here, well off the opponent.
            let pressing = (inp.turret_target.x - cover_x).abs() < 1e-3;
            if pressing {
                t.press_ticks += 1;
                t.first_press.get_or_insert(i as f32 * DT);
                if !was_pressing {
                    t.episodes += 1;
                }
            }
            was_pressing = pressing;
            if inp.fire && me.try_fire(FIRE_COOLDOWN) {
                if pressing {
                    t.rock_shots += 1;
                } else {
                    t.opp_shots += 1;
                }
            }
            me.update(&inp, DT);
            me.tick_timers(DT);
        }
        t
    }

    /// The regression this module was written for: gating the blocked-shot
    /// timer on turret alignment made pressing unreachable for a ship in
    /// motion, because every traverse zeroed the timer. Walled in with no
    /// flank, every difficulty must eventually press.
    #[test]
    fn every_difficulty_presses_when_walled_in() {
        for d in [AiDifficulty::Easy, AiDifficulty::Normal, AiDifficulty::Hard] {
            let t = run(d, &wall(), vec2(6.5, 0.0), 4.0, 30.0);
            assert!(
                t.first_press.is_some(),
                "{d:?} never pressed cover in 30s despite having no flank"
            );
            assert!(
                t.rock_shots > 0,
                "{d:?} aimed at cover but never actually shot it"
            );
        }
    }

    /// Pressing must stay a last resort: given a boulder it can drive around,
    /// the agent should spend its time shooting the opponent, not the rock.
    #[test]
    fn flanking_is_preferred_over_pressing_when_a_flank_exists() {
        for d in [AiDifficulty::Easy, AiDifficulty::Normal, AiDifficulty::Hard] {
            let (w, cover) = boulder();
            let t = run(d, &w, vec2(6.5, 0.0), cover.x, 20.0);
            assert!(
                t.opp_shots > t.rock_shots,
                "{d:?} shot cover ({}) at least as often as the opponent ({}) \
                 despite a flank being available",
                t.rock_shots,
                t.opp_shots
            );
            assert!(
                t.press_frac() < 0.5,
                "{d:?} spent {:.0}% of the fight aiming at a rock it could have driven around",
                100.0 * t.press_frac()
            );
        }
    }

    /// Difficulties are ordered by `press_cover_delay` (Hard 0.1s, Normal
    /// 0.2s, Easy 1.0s); with an identical pin, that ordering should show up
    /// in when each one actually commits.
    #[test]
    fn press_onset_follows_the_difficulty_ordering() {
        let onset = |d| {
            run(d, &wall(), vec2(6.5, 0.0), 4.0, 30.0)
                .first_press
                .expect("walled-in agent must press")
        };
        let (hard, normal, easy) = (
            onset(AiDifficulty::Hard),
            onset(AiDifficulty::Normal),
            onset(AiDifficulty::Easy),
        );
        assert!(
            hard <= normal && normal <= easy,
            "press onset should follow Hard <= Normal <= Easy, got {hard:.2} / {normal:.2} / {easy:.2}"
        );
        for (d, t) in [("Hard", hard), ("Normal", normal), ("Easy", easy)] {
            assert!(
                // `first_press` records the start time of the tick whose `dt`
                // advances the timer across the threshold.
                t + DT
                    >= Tuning::for_difficulty(match d {
                        "Hard" => AiDifficulty::Hard,
                        "Normal" => AiDifficulty::Normal,
                        _ => AiDifficulty::Easy,
                    })
                    .press_cover_delay,
                "{d} pressed at {t:.2}s, sooner than its configured delay"
            );
        }
    }

    /// "Intentional pressure, not erratic rock-shooting": once committed, the
    /// agent should hold cover in a few sustained episodes rather than
    /// flickering between the rock and the opponent frame to frame.
    #[test]
    fn pressing_is_sustained_not_chattering() {
        for d in [AiDifficulty::Easy, AiDifficulty::Normal, AiDifficulty::Hard] {
            let t = run(d, &wall(), vec2(6.5, 0.0), 4.0, 30.0);
            let avg_episode = t.press_ticks as f32 / t.episodes.max(1) as f32 * DT;
            assert!(
                avg_episode > 0.25,
                "{d:?} press episodes average {avg_episode:.3}s over {} episodes — \
                 that reads as chatter, not deliberate pressure",
                t.episodes
            );
        }
    }
}
