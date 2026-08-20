use crate::iso::{screen_angle_to_world_angle, world_angle_to_screen_angle};
use macroquad::prelude::*;

/// Neutral (zeroed) controls come from `Default` — used both for inactive AI
/// tree branches and as the starting point before merging inputs.
#[derive(Default, Clone)]
pub struct PlayerInput {
    pub throttle: f32,
    pub turn: f32,
    /// Lateral thrust, independent of heading: positive strafes right.
    pub strafe: f32,
    pub fire: bool,
    pub fire_rocket: bool,
    /// World-space point the turret should track this frame (see
    /// `Player::turret_angle`). Humans get this filled in from the opponent's
    /// position by the caller; the AI's attack branch drives it with a
    /// lead-predicted aim point.
    pub turret_target: Vec2,
}

pub struct Player {
    pub pos: Vec2,
    pub vel: Vec2,
    pub angle: f32,
    pub radius: f32,
    /// Smoothed forward throttle (eases toward input) — drives the engine dust
    /// trail length so it grows/shrinks gradually rather than snapping.
    pub thrust: f32,
    /// Smoothed exhaust direction (eases toward −heading) — makes the dust trail
    /// swing around with a lag when the ship turns instead of snapping.
    pub exhaust_dir: Vec2,
    pub fire_cooldown: f32,
    pub missile_cooldown: f32,
    pub missile_count: u8,
    pub hit_flash: f32,
    pub camera_shake: f32,
    pub hull: f32,
    pub shield: f32,
    pub shield_recharge_timer: f32,
    pub respawn_timer: f32,
    /// Sustained-fire budget: accumulates per shot, decays over time. Once it
    /// crosses `OVERHEAT_THRESHOLD` the gun locks out (`overheated`) until it
    /// vents back down to `OVERHEAT_THRESHOLD * OVERHEAT_RESUME_FRAC` — separate
    /// from `fire_cooldown`, which stays a constant per-shot rate limiter.
    /// Decay is two-rate (`HEAT_DECAY_RATE` while firable, `HEAT_VENT_RATE`
    /// while locked out); see those constants for why they must differ. The
    /// firable rate additionally waits on `HEAT_COOL_DELAY` worth of idle, so
    /// heat only falls during a real lull in fire, never between shots.
    pub heat: f32,
    pub overheated: bool,
    /// Seconds since the last shot. Ambient cooling waits on this reaching
    /// `HEAT_COOL_DELAY`, so an unbroken stream of fire never cools no matter
    /// how it is paced (see that constant).
    pub heat_idle: f32,
    /// World-space gun facing. Tracks `turret_target` within `TURRET_MAX` of
    /// the hull nose (see `update_turret`) — decoupled from `angle` so a ship
    /// can thrust across the enemy's face and keep the gun on target instead
    /// of the fight collapsing onto a radial line.
    pub turret_angle: f32,
}

impl Player {
    // Top speed, accel, brake, and strafe accel are all scaled 1.5x from the
    // original tuning (7.0 / 20.0 / 30.0 / 15.0) to speed up the action.
    // DAMPING is left alone: ACCEL and MAX_SPEED scale together, so the
    // steady-state speed at full throttle (ACCEL / DAMPING) still saturates
    // MAX_SPEED at the same throttle fraction, and the ship reaches the new
    // (higher) top speed in the same time it used to take to reach the old
    // one.
    pub const MAX_SPEED: f32 = 10.5;
    // Reverse tops out well under forward speed: retreating no longer holds
    // range for free, so backing off is a commitment rather than a stance.
    pub const REVERSE_MAX_SPEED: f32 = Self::MAX_SPEED * 0.4;
    pub const ACCEL: f32 = 30.0;
    pub const BRAKE: f32 = 45.0;
    // Turn rate: 1.5x the original 3.2, then pulled back down 25% (of that
    // 4.8) — snappier than the original but not as twitchy as the first pass.
    pub const TURN_RATE: f32 = 3.6;
    pub const STRAFE_ACCEL: f32 = 22.5;
    // A ship holding max speed can no longer reorient at full rate — taxes a
    // committed sprint (chase or backpedal) alike, opening a window to juke.
    pub const TURN_RATE_SPEED_PENALTY: f32 = 0.5;
    pub const DAMPING: f32 = 2.0;
    // Half-angle of the turret's traverse off the hull nose.
    pub const TURRET_MAX: f32 = 28.0 / 180.0 * std::f32::consts::PI;
    pub const TURRET_TRAVERSE_RATE: f32 = 6.0; // rad/s
    pub const THRUST_SMOOTH_RATE: f32 = 3.5; // dust-trail spool-up/down rate (1/s)
    pub const EXHAUST_TURN_RATE: f32 = 4.5; // dust-trail swing rate when turning (1/s)
    pub const FLASH_DURATION: f32 = 0.45;
    pub const MAX_HULL: f32 = 100.0;
    pub const MAX_SHIELD: f32 = 75.0;
    pub const SHIELD_RECHARGE_DELAY: f32 = 3.0;
    pub const SHIELD_RECHARGE_RATE: f32 = 25.0;
    pub const RESPAWN_DELAY: f32 = 3.0;
    pub const COLLISION_MIN_SPEED: f32 = 1.5; // tiles/s — impacts below this do no damage
    pub const COLLISION_DAMAGE_SCALE: f32 = 10.0; // hull HP per tile/s above the threshold
    pub const MAX_MISSILES: u8 = 2;

    // Gun heat: each shot adds `HEAT_PER_SHOT` (matching bullet damage, so the
    // threshold reads directly in "equivalent kills"); overheat threshold is a
    // single tunable multiple of a full-health kill's total damage pool.
    pub const HEAT_PER_SHOT: f32 = crate::bullet::Bullet::DAMAGE;
    // Tuned for ~1s of held fire before lockout. Because `HEAT_COOL_DELAY`
    // suppresses decay entirely for the duration of a burst, heat during one
    // is just `HEAT_PER_SHOT` per shot, so this is simply "how many shots fit
    // before lockout" — 54.25 trips on the 6th. That also makes the shot count
    // frame-rate independent (no decay term to vary); only the wall-clock
    // onset shifts slightly with `FIRE_COOLDOWN`'s frame quantization. Keep it
    // mid-band between two shot boundaries (here 50 and 60) so the same shot
    // trips it at every refresh rate.
    pub const OVERHEAT_KILL_MULTIPLIER: f32 = 0.31;
    pub const OVERHEAT_THRESHOLD: f32 =
        Self::OVERHEAT_KILL_MULTIPLIER * (Self::MAX_HULL + Self::MAX_SHIELD);
    // Hysteresis resume point: once overheated, the gun stays locked until
    // heat decays back down to this fraction of the threshold — otherwise the
    // gun would stutter fire right at the threshold edge.
    pub const OVERHEAT_RESUME_FRAC: f32 = 0.3;
    // Ambient heat/s bled off once the gun has been idle for `HEAT_COOL_DELAY`.
    // Must stay below a held trigger's gain of `HEAT_PER_SHOT / FIRE_COOLDOWN`
    // = 55.6 heat/s or the threshold would be unreachable and the mechanic
    // inert. Together with the delay this sets the *sustainable* rate of fire:
    // cooling only outpaces a shot once the gap between shots exceeds
    // `HEAT_COOL_DELAY + HEAT_PER_SHOT / HEAT_DECAY_RATE` (~1.1s).
    pub const HEAT_DECAY_RATE: f32 = 20.0;
    // Idle seconds required before ambient cooling starts. Without this, decay
    // runs between every shot and a player can pace fire to sit just under the
    // threshold indefinitely — sustaining ~2 shots/s forever and sidestepping
    // the mechanic entirely. Gating cooling behind a real pause means any
    // uninterrupted stream accumulates no matter how it is spaced, so the
    // counterplay is taking an actual break rather than metering the trigger.
    // Deliberately longer than `FIRE_COOLDOWN` (0.18s), or every burst would
    // cool between its own shots and nothing would change.
    pub const HEAT_COOL_DELAY: f32 = 0.6;
    // Heat/s vented while locked out, tuned for a ~0.5s lockout. Much faster
    // than ambient decay again (the two rates cannot be merged: ambient must
    // stay slow enough for heat to accumulate at all), and unlike ambient
    // decay it ignores `HEAT_COOL_DELAY` — a lockout is already a forced
    // pause, so making it wait to start venting would just stretch it. Note
    // the vent starts from wherever the latching shot overshot the threshold
    // (60, not 54.25), which is why this sits above the naive
    // (54.25 - 16.3) / 0.5s.
    pub const HEAT_VENT_RATE: f32 = 87.0;

    pub fn new(pos: Vec2) -> Self {
        Self {
            pos,
            vel: Vec2::ZERO,
            angle: 0.0,
            turret_angle: 0.0,
            radius: 0.65,
            thrust: 0.0,
            exhaust_dir: vec2(-1.0, 0.0), // opposite the initial heading (angle 0)
            fire_cooldown: 0.0,
            missile_cooldown: 0.0,
            missile_count: Self::MAX_MISSILES,
            hit_flash: 0.0,
            camera_shake: 0.0,
            hull: Self::MAX_HULL,
            shield: Self::MAX_SHIELD,
            // Start with the recharge delay already elapsed so shield is full.
            shield_recharge_timer: Self::SHIELD_RECHARGE_DELAY,
            respawn_timer: 0.0,
            heat: 0.0,
            overheated: false,
            // Starts "long idle" so a fresh ship isn't briefly barred from
            // cooling before its first shot.
            heat_idle: Self::HEAT_COOL_DELAY,
        }
    }

    #[inline]
    pub fn is_dead(&self) -> bool {
        self.respawn_timer > 0.0
    }

    pub fn apply_damage(&mut self, amount: f32) {
        self.hit_flash = Self::FLASH_DURATION;
        self.shield_recharge_timer = 0.0;
        self.camera_shake = (self.camera_shake + amount * 0.05).min(1.5);
        if self.shield > 0.0 {
            let absorbed = self.shield.min(amount);
            self.shield -= absorbed;
            let leftover = amount - absorbed;
            if leftover > 0.0 {
                self.hull -= leftover;
            }
        } else {
            self.hull -= amount;
        }
        self.hull = self.hull.max(0.0);
    }

    /// The gun's firing gate: consumes a shot if the per-shot rate limiter has
    /// expired and the gun isn't locked out, charging `HEAT_PER_SHOT` and
    /// latching `overheated` on the way. Both controllers go through here, so
    /// heat applies uniformly to humans and AI (see `game.rs`'s spawn loop).
    pub fn try_fire(&mut self, cooldown: f32) -> bool {
        if self.fire_cooldown > 0.0 || self.overheated {
            return false;
        }
        self.fire_cooldown = cooldown;
        self.heat += Self::HEAT_PER_SHOT;
        self.heat_idle = 0.0;
        if self.heat >= Self::OVERHEAT_THRESHOLD {
            self.overheated = true;
        }
        true
    }

    pub fn tick_timers(&mut self, dt: f32) {
        self.fire_cooldown = (self.fire_cooldown - dt).max(0.0);
        self.missile_cooldown = (self.missile_cooldown - dt).max(0.0);
        self.hit_flash = (self.hit_flash - dt).max(0.0);
        self.camera_shake = (self.camera_shake - dt * 8.0).max(0.0);

        self.heat_idle += dt;
        // A lockout vents unconditionally — it is already a forced pause, so
        // it must not also wait out the idle delay. Otherwise cooling only
        // starts once the gun has genuinely been quiet for `HEAT_COOL_DELAY`.
        let decay = if self.overheated {
            Self::HEAT_VENT_RATE
        } else if self.heat_idle >= Self::HEAT_COOL_DELAY {
            Self::HEAT_DECAY_RATE
        } else {
            0.0
        };
        self.heat = (self.heat - decay * dt).max(0.0);
        if self.overheated && self.heat <= Self::OVERHEAT_THRESHOLD * Self::OVERHEAT_RESUME_FRAC {
            self.overheated = false;
        }

        self.shield_recharge_timer += dt;
        if self.shield_recharge_timer > Self::SHIELD_RECHARGE_DELAY
            && self.shield < Self::MAX_SHIELD
        {
            self.shield = (self.shield + Self::SHIELD_RECHARGE_RATE * dt).min(Self::MAX_SHIELD);
        }
    }

    pub fn update(&mut self, input: &PlayerInput, dt: f32) {
        // miniquad's frame timer is wall-clock, not monotonic, so a clock
        // step can hand us a negative dt — which would panic the symmetric
        // `.clamp(-rate * dt, rate * dt)` in `update_turret` below (min > max).
        let dt = dt.max(0.0);

        // Speed-scaled turn rate: a ship holding max speed — chasing or
        // backpedaling — can't reorient at full rate, so the other side can
        // juke past it. Fraction of the *forward* MAX_SPEED, not whichever cap
        // currently applies, so a full-reverse ship keeps most of its agility.
        let speed_frac = (self.vel.length() / Self::MAX_SPEED).min(1.0);
        let turn_rate = Self::TURN_RATE * (1.0 - Self::TURN_RATE_SPEED_PENALTY * speed_frac);

        // Turn in screen space so the visual rate is uniform regardless of heading.
        // The iso projection (TW=64, TH=32) stretches angles non-uniformly; turning
        // in world space would make the ship appear to rotate faster on some axes.
        let screen_angle = world_angle_to_screen_angle(self.angle);
        self.angle = screen_angle_to_world_angle(screen_angle + input.turn * turn_rate * dt);

        // Ease the visual thrust toward the input so the dust trail spools up and
        // down smoothly. Frame-rate independent exponential smoothing (~0.3s τ).
        let smooth = 1.0 - (-Self::THRUST_SMOOTH_RATE * dt).exp();
        self.thrust += (input.throttle - self.thrust) * smooth;

        let heading = vec2(self.angle.cos(), self.angle.sin());

        // Lag the exhaust direction behind the hull so the dust trail swings
        // around after a turn. Eased toward −heading; the shader normalises it.
        let turn_smooth = 1.0 - (-Self::EXHAUST_TURN_RATE * dt).exp();
        self.exhaust_dir += (-heading - self.exhaust_dir) * turn_smooth;

        let accel = if input.throttle >= 0.0 {
            Self::ACCEL
        } else {
            Self::BRAKE
        };
        self.vel += heading * (input.throttle * accel) * dt;

        let strafe_dir = vec2(-heading.y, heading.x);
        self.vel += strafe_dir * (input.strafe * Self::STRAFE_ACCEL) * dt;

        self.vel *= (1.0 - Self::DAMPING * dt).max(0.0);

        // Reverse speed cap: net backward motion relative to the nose tops
        // out well under the forward top speed (see REVERSE_MAX_SPEED) — a
        // kiter can no longer hold range against a charger for free.
        let forward_speed = self.vel.dot(heading);
        let max_speed = if forward_speed < 0.0 {
            Self::REVERSE_MAX_SPEED
        } else {
            Self::MAX_SPEED
        };
        let s = self.vel.length();
        if s > max_speed {
            self.vel *= max_speed / s;
        }

        self.pos += self.vel * dt;

        self.update_turret(input.turret_target, dt);
    }

    // Traverse the turret toward `target`, clamped to `TURRET_MAX` off the
    // hull nose and rate-limited by `TURRET_TRAVERSE_RATE`. Re-clamped every
    // call (not just eased toward a clamped setpoint) so a hull that snaps
    // around under a turret sitting near the limit can't leave it stranded
    // outside the cone.
    fn update_turret(&mut self, target: Vec2, dt: f32) {
        let to_target = target - self.pos;
        if to_target.length_squared() > 1e-6 {
            let desired = to_target.y.atan2(to_target.x);
            let rel = wrap_pi(desired - self.angle).clamp(-Self::TURRET_MAX, Self::TURRET_MAX);
            let desired_turret = self.angle + rel;
            let step = wrap_pi(desired_turret - self.turret_angle).clamp(
                -Self::TURRET_TRAVERSE_RATE * dt,
                Self::TURRET_TRAVERSE_RATE * dt,
            );
            self.turret_angle = wrap_pi(self.turret_angle + step);
        }
        let rel =
            wrap_pi(self.turret_angle - self.angle).clamp(-Self::TURRET_MAX, Self::TURRET_MAX);
        self.turret_angle = wrap_pi(self.angle + rel);
    }
}

fn wrap_pi(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut a = a % TAU;
    if a > PI {
        a -= TAU;
    } else if a < -PI {
        a += TAU;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The per-shot rate limiter `game.rs` passes to `try_fire`. Mirrored here
    /// so the heat math below is checked against the real firing cadence.
    const FIRE_COOLDOWN: f32 = 0.18;

    /// Hold the trigger down for `duty` of every second and run the real gate
    /// (`try_fire`) + real decay (`tick_timers`) for `secs`. Returns the peak
    /// heat reached and the time of the first overheat, if any.
    fn sim_duty_cycle(duty: f32, secs: f32) -> (f32, Option<f32>) {
        let dt = 1.0 / 60.0;
        let mut p = Player::new(Vec2::ZERO);
        let (mut peak, mut first) = (0.0f32, None);
        for i in 0..(secs / dt) as i32 {
            let t = i as f32 * dt;
            if (t % 1.0) < duty {
                p.try_fire(FIRE_COOLDOWN);
            }
            if p.overheated && first.is_none() {
                first = Some(t);
            }
            p.tick_timers(dt);
            peak = peak.max(p.heat);
        }
        (peak, first)
    }

    /// Regression guard for the constant that decides whether the mechanic
    /// exists at all: ambient decay must lose to a held trigger's heat gain,
    /// or the threshold is simply unreachable and the gun never overheats.
    #[test]
    fn ambient_decay_is_slower_than_held_trigger_gain() {
        let gain_per_sec = Player::HEAT_PER_SHOT / FIRE_COOLDOWN;
        assert!(
            Player::HEAT_DECAY_RATE < gain_per_sec,
            "ambient decay {} >= held-trigger gain {gain_per_sec}: overheat unreachable",
            Player::HEAT_DECAY_RATE
        );
    }

    #[test]
    fn held_trigger_overheats_within_a_few_seconds() {
        let (_, first) = sim_duty_cycle(1.0, 30.0);
        let t = first.expect("a held trigger must overheat");
        assert!(
            (0.7..1.3).contains(&t),
            "held trigger overheated at {t}s, outside the intended ~1s (0.7-1.3s) window"
        );
    }

    /// Fire one shot every `gap` seconds for `secs`, through the real gate and
    /// decay. Returns when the gun first overheated, if it did.
    fn sim_paced_fire(gap: f32, secs: f32) -> Option<f32> {
        let dt = 1.0 / 60.0;
        let mut p = Player::new(Vec2::ZERO);
        let mut since = f32::MAX;
        for i in 0..(secs / dt).round() as i32 {
            if since >= gap && p.try_fire(FIRE_COOLDOWN) {
                since = 0.0;
            }
            if p.overheated {
                return Some(i as f32 * dt);
            }
            p.tick_timers(dt);
            since += dt;
        }
        None
    }

    /// The point of `HEAT_COOL_DELAY`: metering the trigger to sit just under
    /// the threshold used to be sustainable forever, which sidestepped the
    /// mechanic. Any stream this tight must now cook the gun eventually.
    #[test]
    fn metered_fire_no_longer_dodges_overheating() {
        for gap in [0.3, 0.5, 0.7, 0.9] {
            assert!(
                sim_paced_fire(gap, 60.0).is_some(),
                "firing every {gap}s sustained indefinitely — the pacing exploit is back"
            );
        }
    }

    /// The counterplay has to remain real: pause long enough for cooling to
    /// actually start and outrun a shot, and fire stays sustainable.
    #[test]
    fn a_genuine_pause_still_sustains_fire() {
        let sustainable = Player::HEAT_COOL_DELAY + Player::HEAT_PER_SHOT / Player::HEAT_DECAY_RATE;
        for gap in [sustainable + 0.05, sustainable + 0.5, sustainable + 1.0] {
            assert_eq!(
                sim_paced_fire(gap, 60.0),
                None,
                "firing every {gap}s overheated, but cooling should outpace it"
            );
        }
    }

    /// Cooling must not start between the shots of a single burst, or the
    /// delay does nothing at all.
    #[test]
    fn cooling_does_not_start_between_shots_of_a_burst() {
        // Const-asserted: a cool delay inside the per-shot cadence would let a
        // burst cool between its own shots, making the delay a no-op.
        const { assert!(Player::HEAT_COOL_DELAY > FIRE_COOLDOWN) };
        let dt = 1.0 / 60.0;
        let mut p = Player::new(Vec2::ZERO);
        p.try_fire(FIRE_COOLDOWN);
        let after_first = p.heat;
        // Idle for just under the delay: heat must be untouched.
        for _ in 0..((Player::HEAT_COOL_DELAY - 2.0 * dt) / dt).round() as i32 {
            p.tick_timers(dt);
        }
        assert_eq!(
            p.heat, after_first,
            "heat cooled before the idle delay elapsed"
        );
        // Past the delay it finally bleeds.
        for _ in 0..30 {
            p.tick_timers(dt);
        }
        assert!(
            p.heat < after_first,
            "heat never cooled after the idle delay"
        );
    }

    #[test]
    fn lockout_vents_and_clears_at_the_resume_point() {
        let mut p = Player::new(Vec2::ZERO);
        p.heat = Player::OVERHEAT_THRESHOLD;
        p.overheated = true;

        let dt = 1.0 / 60.0;
        let mut elapsed = 0.0;
        while p.overheated && elapsed < 30.0 {
            assert!(!p.try_fire(FIRE_COOLDOWN), "fired while locked out");
            p.tick_timers(dt);
            elapsed += dt;
        }
        assert!(!p.overheated, "lockout never cleared");
        assert!(
            (0.3..0.8).contains(&elapsed),
            "lockout lasted {elapsed}s, outside the intended ~0.5s (0.3-0.8s) window"
        );
        assert!(
            p.try_fire(FIRE_COOLDOWN),
            "gun did not resume after venting"
        );
    }

    /// Hysteresis: clearing at the threshold itself would let the gun stutter
    /// one shot back into lockout every frame.
    #[test]
    fn overheat_clears_below_the_threshold_not_at_it() {
        const { assert!(Player::OVERHEAT_RESUME_FRAC < 1.0) };
        let mut p = Player::new(Vec2::ZERO);
        p.heat = Player::OVERHEAT_THRESHOLD;
        p.overheated = true;
        p.tick_timers(1.0 / 60.0);
        assert!(p.overheated, "cleared immediately with no hysteresis band");
    }
}
