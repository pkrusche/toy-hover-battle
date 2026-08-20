use crate::player::PlayerInput;

// ── Menu navigation ───────────────────────────────────────────────────────────
// Menus reuse the dpad for navigation and the A/B face buttons to
// confirm/back, rather than a separate mapping, so a player never has to
// learn dedicated menu controls just to get to the game.

/// Edge-triggered per-frame menu controls, aggregated across every connected
/// pad. Edge-triggered (not held) so a single dpad tap moves the selection by
/// exactly one step, matching `is_key_pressed` for the keyboard.
#[derive(Default, Clone, Copy)]
pub struct MenuInput {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    /// Either fire button (gun or rocket) confirms — the task only needs a
    /// single "select", and requiring one specific button would be an
    /// arbitrary restriction players would have to discover by trial.
    pub confirm: bool,
    /// Start/Menu button — mirrors what Escape does on each screen: quits
    /// from the startup screen, backs out of a submenu.
    pub back: bool,
}

/// Previous-frame button state for one pad's dpad + A/B/Start, used to turn
/// level signals into edges. Kept separate from gameplay's own edge tracking
/// (`fire_rocket_pending` / macOS `prev_rocket`) so menu navigation and
/// in-match input never interfere with each other's edge detection.
#[derive(Default, Clone, Copy)]
struct DpadEdge {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    a: bool,
    b: bool,
    start: bool,
}

impl MenuInput {
    fn accumulate(&mut self, prev: DpadEdge, now: DpadEdge) {
        self.up |= now.up && !prev.up;
        self.down |= now.down && !prev.down;
        self.left |= now.left && !prev.left;
        self.right |= now.right && !prev.right;
        self.confirm |= (now.a && !prev.a) || (now.b && !prev.b);
        self.back |= now.start && !prev.start;
    }
}

// ── macOS: Apple GameController framework ────────────────────────────────────
// gilrs uses IOKit HID directly and cannot decode the Switch Pro Controller's
// proprietary report format. GCController abstracts over it correctly.

#[cfg(target_os = "macos")]
#[link(name = "GameController", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
use objc::{class, msg_send, runtime::Object, sel, sel_impl};

// GCControllerPlayerIndex: -1 = unset, 0 = P1, 1 = P2, 2 = P3, 3 = P4.
// We tag each controller with the player slot on first assignment and look it
// up by tag in merge_into, so array reordering on connect/disconnect is
// invisible to the game.
#[cfg(target_os = "macos")]
const GC_UNSET: isize = -1;

#[cfg(target_os = "macos")]
pub struct Pads {
    prev_slots_filled: [bool; 2],
    prev_rocket: [bool; 2],
    menu_prev: [DpadEdge; 2],
}

#[cfg(target_os = "macos")]
impl Pads {
    pub fn new() -> Self {
        let mut prev_slots_filled = [false; 2];
        unsafe {
            let arr: *mut Object = msg_send![class!(GCController), controllers];
            let n: usize = msg_send![arr, count];
            for (slot, i) in (0..n).take(2).enumerate() {
                let c: *mut Object = msg_send![arr, objectAtIndex: i];
                let _: () = msg_send![c, setPlayerIndex: slot as isize];
                println!("gamepad P{} = {}", slot + 1, gc_vendor(c));
                prev_slots_filled[slot] = true;
            }
        }
        Self {
            prev_slots_filled,
            prev_rocket: [false; 2],
            menu_prev: [DpadEdge::default(); 2],
        }
    }

    pub fn update(&mut self) {
        unsafe {
            let arr: *mut Object = msg_send![class!(GCController), controllers];
            let n: usize = msg_send![arr, count];

            // Which player slots have a live controller this frame.
            let mut slot_filled = [false; 2];
            for i in 0..n {
                let c: *mut Object = msg_send![arr, objectAtIndex: i];
                let pidx: isize = msg_send![c, playerIndex];
                if pidx >= 0 && (pidx as usize) < 2 {
                    slot_filled[pidx as usize] = true;
                }
            }

            // Log disconnections.
            for (s, (&was_filled, &is_filled)) in self
                .prev_slots_filled
                .iter()
                .zip(slot_filled.iter())
                .enumerate()
            {
                if was_filled && !is_filled {
                    println!("gamepad P{} disconnected", s + 1);
                }
            }

            // Assign newly connected controllers (playerIndex == GC_UNSET) to
            // the first free slot, preserving existing assignments.
            for i in 0..n {
                let c: *mut Object = msg_send![arr, objectAtIndex: i];
                let pidx: isize = msg_send![c, playerIndex];
                if pidx == GC_UNSET {
                    if let Some(s) = slot_filled.iter().position(|filled| !filled) {
                        let _: () = msg_send![c, setPlayerIndex: s as isize];
                        println!("gamepad P{} connected: {}", s + 1, gc_vendor(c));
                        slot_filled[s] = true;
                    }
                }
            }

            self.prev_slots_filled = slot_filled;
        }
    }

    pub fn merge_into(&mut self, idx: usize, input: &mut PlayerInput) {
        unsafe {
            // Find the controller tagged with this player's slot index.
            let arr: *mut Object = msg_send![class!(GCController), controllers];
            let n: usize = msg_send![arr, count];
            let mut ctrl: *mut Object = std::ptr::null_mut();
            for i in 0..n {
                let c: *mut Object = msg_send![arr, objectAtIndex: i];
                let pidx: isize = msg_send![c, playerIndex];
                if pidx == idx as isize {
                    ctrl = c;
                    break;
                }
            }
            if ctrl.is_null() {
                return;
            }

            let gp: *mut Object = msg_send![ctrl, extendedGamepad];
            if gp.is_null() {
                return;
            }

            let dpad: *mut Object = msg_send![gp, dpad];
            let dpad_x: *mut Object = msg_send![dpad, xAxis];
            let dpad_y: *mut Object = msg_send![dpad, yAxis];
            let throttle: f32 = msg_send![dpad_y, value];
            let turn: f32 = msg_send![dpad_x, value];

            // Strafe on the front (top) shoulder pair, gun on the A face
            // button, rocket on the B face button.
            let right_shoulder: *mut Object = msg_send![gp, rightShoulder];
            let rs_val: f32 = msg_send![right_shoulder, value];

            let left_shoulder: *mut Object = msg_send![gp, leftShoulder];
            let ls_val: f32 = msg_send![left_shoulder, value];

            let btn_a: *mut Object = msg_send![gp, buttonA];
            let a_val: f32 = msg_send![btn_a, value];

            let btn_b: *mut Object = msg_send![gp, buttonB];
            let b_val: f32 = msg_send![btn_b, value];
            let b_now = b_val > 0.5;

            input.throttle = (input.throttle + throttle).clamp(-1.0, 1.0);
            input.turn = (input.turn + turn).clamp(-1.0, 1.0);
            input.strafe = (input.strafe + (rs_val - ls_val)).clamp(-1.0, 1.0);
            input.fire |= a_val > 0.5;
            input.fire_rocket |= b_now && !self.prev_rocket[idx];
            self.prev_rocket[idx] = b_now;
        }
    }

    /// Re-baselines `merge_into`'s B-button edge tracking against the pad's
    /// current state. Call this right before a match starts: `prev_rocket`
    /// only advances inside `merge_into`, which isn't called while a menu is
    /// up, so a controller's B button still held down from confirming
    /// "Start" would otherwise read as a fresh press on the match's first
    /// frame and fire a rocket nobody asked for.
    pub fn reset_match_input(&mut self) {
        unsafe {
            let arr: *mut Object = msg_send![class!(GCController), controllers];
            let n: usize = msg_send![arr, count];
            for i in 0..n {
                let c: *mut Object = msg_send![arr, objectAtIndex: i];
                let pidx: isize = msg_send![c, playerIndex];
                if pidx < 0 || pidx as usize >= 2 {
                    continue;
                }
                let gp: *mut Object = msg_send![c, extendedGamepad];
                if gp.is_null() {
                    continue;
                }
                let btn_b: *mut Object = msg_send![gp, buttonB];
                let b_val: f32 = msg_send![btn_b, value];
                self.prev_rocket[pidx as usize] = b_val > 0.5;
            }
        }
    }

    /// Aggregates dpad + A/B edges from every assigned controller into a
    /// single menu gesture set for this frame.
    pub fn menu_input(&mut self) -> MenuInput {
        let mut out = MenuInput::default();
        unsafe {
            let arr: *mut Object = msg_send![class!(GCController), controllers];
            let n: usize = msg_send![arr, count];
            for i in 0..n {
                let c: *mut Object = msg_send![arr, objectAtIndex: i];
                let pidx: isize = msg_send![c, playerIndex];
                if pidx < 0 || pidx as usize >= 2 {
                    continue;
                }
                let slot = pidx as usize;

                let gp: *mut Object = msg_send![c, extendedGamepad];
                if gp.is_null() {
                    continue;
                }

                let dpad: *mut Object = msg_send![gp, dpad];
                let dpad_x: *mut Object = msg_send![dpad, xAxis];
                let dpad_y: *mut Object = msg_send![dpad, yAxis];
                let x: f32 = msg_send![dpad_x, value];
                let y: f32 = msg_send![dpad_y, value];

                let btn_a: *mut Object = msg_send![gp, buttonA];
                let btn_b: *mut Object = msg_send![gp, buttonB];
                let a_val: f32 = msg_send![btn_a, value];
                let b_val: f32 = msg_send![btn_b, value];

                // buttonMenu (Start/Options) is a GCExtendedGamepad property,
                // not a GCController one, and only exists on macOS 11+ — on
                // an older OS the selector is unrecognized, and an
                // Objective-C exception unwinding into Rust is an abort, not
                // a catchable error, so this must be checked before sending.
                let responds_to_menu: bool = msg_send![gp, respondsToSelector: sel!(buttonMenu)];
                let start = if responds_to_menu {
                    let btn_menu: *mut Object = msg_send![gp, buttonMenu];
                    if btn_menu.is_null() {
                        false
                    } else {
                        let menu_val: f32 = msg_send![btn_menu, value];
                        menu_val > 0.5
                    }
                } else {
                    false
                };

                let now = DpadEdge {
                    up: y > 0.5,
                    down: y < -0.5,
                    left: x < -0.5,
                    right: x > 0.5,
                    a: a_val > 0.5,
                    b: b_val > 0.5,
                    start,
                };
                out.accumulate(self.menu_prev[slot], now);
                self.menu_prev[slot] = now;
            }
        }
        out
    }

    /// True while any assigned controller's Start/Menu button is currently
    /// held, for driving the same hold-to-quit gesture as Escape during a
    /// match — unlike `menu_input`'s `back`, this is level- not edge-
    /// triggered, since a hold needs to see the button down every frame.
    pub fn start_held(&self) -> bool {
        unsafe {
            let arr: *mut Object = msg_send![class!(GCController), controllers];
            let n: usize = msg_send![arr, count];
            for i in 0..n {
                let c: *mut Object = msg_send![arr, objectAtIndex: i];
                let pidx: isize = msg_send![c, playerIndex];
                if pidx < 0 || pidx as usize >= 2 {
                    continue;
                }
                let gp: *mut Object = msg_send![c, extendedGamepad];
                if gp.is_null() {
                    continue;
                }
                let responds_to_menu: bool = msg_send![gp, respondsToSelector: sel!(buttonMenu)];
                if !responds_to_menu {
                    continue;
                }
                let btn_menu: *mut Object = msg_send![gp, buttonMenu];
                if btn_menu.is_null() {
                    continue;
                }
                let menu_val: f32 = msg_send![btn_menu, value];
                if menu_val > 0.5 {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(target_os = "macos")]
unsafe fn gc_vendor(c: *mut Object) -> String {
    let ns: *mut Object = msg_send![c, vendorName];
    if ns.is_null() {
        return "Unknown".into();
    }
    let ptr: *const std::ffi::c_char = msg_send![ns, UTF8String];
    std::ffi::CStr::from_ptr(ptr)
        .to_str()
        .unwrap_or("Unknown")
        .to_owned()
}

// ── non-macOS: gilrs ──────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
use gilrs::{Button, EventType, GamepadId, Gilrs};

#[cfg(not(target_os = "macos"))]
pub struct Pads {
    /// `None` when the input backend refused to start — the game then runs
    /// keyboard-only instead of failing before the menu.
    gilrs: Option<Gilrs>,
    assigned: [Option<GamepadId>; 2],
    fire_rocket_pending: [bool; 2],
    menu_prev: [DpadEdge; 2],
}

#[cfg(not(target_os = "macos"))]
impl Pads {
    pub fn new() -> Self {
        // A platform with no gamepad backend still hands back a usable
        // (permanently empty) Gilrs; anything else means no gamepad support
        // this run, which must not stop a keyboard-only player.
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(gilrs::Error::NotImplemented(gilrs)) => {
                println!("gamepad support unavailable on this platform");
                Some(gilrs)
            }
            Err(err) => {
                println!("gamepad support disabled: {err}");
                None
            }
        };
        let mut assigned: [Option<GamepadId>; 2] = [None; 2];
        for (id, pad) in gilrs.iter().flat_map(|gilrs| gilrs.gamepads()) {
            if let Some((i, slot)) = assigned.iter_mut().enumerate().find(|(_, s)| s.is_none()) {
                *slot = Some(id);
                println!("gamepad P{} assigned: {}", i + 1, pad.name());
            }
        }
        Self {
            gilrs,
            assigned,
            fire_rocket_pending: [false; 2],
            menu_prev: [DpadEdge::default(); 2],
        }
    }

    pub fn update(&mut self) {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return;
        };
        while let Some(ev) = gilrs.next_event() {
            match ev.event {
                EventType::Connected => {
                    let name = gilrs.gamepad(ev.id).name().to_string();
                    if !self.assigned.contains(&Some(ev.id)) {
                        if let Some((i, slot)) = self
                            .assigned
                            .iter_mut()
                            .enumerate()
                            .find(|(_, s)| s.is_none())
                        {
                            *slot = Some(ev.id);
                            println!("gamepad P{} assigned: {}", i + 1, name);
                        }
                    }
                }
                EventType::Disconnected => {
                    for (i, slot) in self.assigned.iter_mut().enumerate() {
                        if *slot == Some(ev.id) {
                            *slot = None;
                            println!("gamepad P{} disconnected", i + 1);
                        }
                    }
                }
                // Rocket sits on the B face button; gun is the A face
                // button, read as a level in `merge_into`.
                EventType::ButtonPressed(Button::East, _) => {
                    for (i, slot) in self.assigned.iter().enumerate() {
                        if *slot == Some(ev.id) {
                            self.fire_rocket_pending[i] = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub fn merge_into(&mut self, idx: usize, input: &mut PlayerInput) {
        let Some(id) = self.assigned[idx] else { return };
        let Some(gilrs) = self.gilrs.as_ref() else {
            return;
        };

        let (throttle, turn, strafe, fire) = {
            let pad = gilrs.gamepad(id);
            let throttle = pad.is_pressed(Button::DPadUp) as i32 as f32
                - pad.is_pressed(Button::DPadDown) as i32 as f32;
            let turn = pad.is_pressed(Button::DPadRight) as i32 as f32
                - pad.is_pressed(Button::DPadLeft) as i32 as f32;
            // Strafe on the front (top) shoulder pair, gun on the A face
            // button.
            let strafe = pad.is_pressed(Button::RightTrigger) as i32 as f32
                - pad.is_pressed(Button::LeftTrigger) as i32 as f32;
            (throttle, turn, strafe, pad.is_pressed(Button::South))
        };

        input.throttle = (input.throttle + throttle).clamp(-1.0, 1.0);
        input.turn = (input.turn + turn).clamp(-1.0, 1.0);
        input.strafe = (input.strafe + strafe).clamp(-1.0, 1.0);
        input.fire |= fire;
        input.fire_rocket |= self.fire_rocket_pending[idx];
        self.fire_rocket_pending[idx] = false;
    }

    /// Discards any rocket-fire button press queued while a menu was up.
    /// Call this right before a match starts: `fire_rocket_pending` is set by
    /// the B-button-pressed event and only drained by `merge_into`, which
    /// isn't called while a menu is up — so confirming "Start" with the
    /// rocket button would otherwise carry that press into the match and
    /// fire a rocket nobody asked for.
    pub fn reset_match_input(&mut self) {
        self.fire_rocket_pending = [false; 2];
    }

    /// Aggregates dpad + A/B edges from every assigned controller into a
    /// single menu gesture set for this frame.
    pub fn menu_input(&mut self) -> MenuInput {
        let mut out = MenuInput::default();
        let Some(gilrs) = self.gilrs.as_ref() else {
            return out;
        };
        for slot in 0..2 {
            let Some(id) = self.assigned[slot] else {
                continue;
            };
            let pad = gilrs.gamepad(id);
            let now = DpadEdge {
                up: pad.is_pressed(Button::DPadUp),
                down: pad.is_pressed(Button::DPadDown),
                left: pad.is_pressed(Button::DPadLeft),
                right: pad.is_pressed(Button::DPadRight),
                a: pad.is_pressed(Button::South),
                b: pad.is_pressed(Button::East),
                start: pad.is_pressed(Button::Start),
            };
            out.accumulate(self.menu_prev[slot], now);
            self.menu_prev[slot] = now;
        }
        out
    }

    /// True while any assigned controller's Start button is currently held,
    /// for driving the same hold-to-quit gesture as Escape during a match —
    /// unlike `menu_input`'s `back`, this is level- not edge-triggered, since
    /// a hold needs to see the button down every frame.
    pub fn start_held(&self) -> bool {
        let Some(gilrs) = self.gilrs.as_ref() else {
            return false;
        };
        self.assigned
            .iter()
            .flatten()
            .any(|&id| gilrs.gamepad(id).is_pressed(Button::Start))
    }
}
