// Release builds are GUI apps on Windows: without this the .exe is a console
// subsystem binary and every player gets a stray terminal behind the game.
// Debug builds keep the console so the gamepad diagnostics stay visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use macroquad::miniquad::conf::Icon;
use macroquad::prelude::*;
use macroquad::window::{get_internal_gl, miniquad::Backend};

mod ai;
mod assets;
mod audio;
mod background;
mod bullet;
mod game;
mod iso;
mod pads;
mod player;
mod world;

use ai::{AiDifficulty, ControllerKind};
use audio::{Sfx, SfxEvent};
use background::{Background, BackgroundObstacles, BackgroundView, BiomeParams};
use game::{GameSpeed, GameState, Layout, VH, VW, VW_FULL};

#[derive(PartialEq)]
enum AppScreen {
    Startup,
    Setup,
    Help,
    Playing,
}

// Returns true if the mouse was clicked inside rect this frame.
fn clicked(rect: Rect) -> bool {
    is_mouse_button_pressed(MouseButton::Left) && rect.contains(mouse_position().into())
}

// How long the quit chord must be held to abandon a match. Long enough that a
// stray Escape mid-fight can't dump both players back to the menu, short
// enough not to feel like the key is broken.
const EXIT_HOLD_TIME: f32 = 0.9;

// Escape leaves a match only when *held* — see `HoldToConfirm` for why.
fn exit_hold_down() -> bool {
    is_key_down(KeyCode::Escape)
}

// Ctrl+C leaves a match immediately: it's a deliberate two-key chord, so it
// can't be hit by a stray elbow the way a lone Escape can, and anyone reaching
// for the terminal habit expects it to bite at once. Edge-triggered on `C`
// (`is_key_pressed`) with the modifier merely held, so holding the chord fires
// exactly once rather than every frame.
fn exit_now_pressed() -> bool {
    let ctrl = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
    ctrl && is_key_pressed(KeyCode::C)
}

/// Accumulates held time for a hold-to-confirm gesture. Progress is all-or-
/// nothing: letting go before the end discards it rather than draining, so a
/// tap can never inch the gesture toward firing.
#[derive(Default)]
struct HoldToConfirm {
    held: f32,
}

impl HoldToConfirm {
    /// Advance by `dt`. Returns true on the one frame the hold completes, then
    /// re-arms from zero (callers act on that frame, so a still-held key would
    /// have to complete another full hold to fire again).
    fn tick(&mut self, down: bool, dt: f32, duration: f32) -> bool {
        if !down {
            self.held = 0.0;
            return false;
        }
        self.held += dt;
        if self.held >= duration {
            self.held = 0.0;
            return true;
        }
        false
    }

    fn in_progress(&self) -> bool {
        self.held > 0.0
    }

    fn progress(&self, duration: f32) -> f32 {
        (self.held / duration).clamp(0.0, 1.0)
    }

    fn reset(&mut self) {
        self.held = 0.0;
    }
}

// Progress ring + prompt for the hold-to-quit chord, drawn over the match.
// Without this the first half-second of a long-press is indistinguishable from
// a dead key, and players conclude Escape doesn't work.
fn draw_exit_hold_prompt(progress: f32) {
    let (cx, cy) = (screen_width() * 0.5, screen_height() * 0.22);
    let label = "hold to quit";
    let font_size = 20.0_f32;
    let d = measure_text(label, None, font_size as u16, 1.0);

    let pad = 12.0;
    let (bw, bh) = (d.width + pad * 2.0, d.height + pad * 2.0);
    draw_rectangle(
        cx - bw * 0.5,
        cy - bh * 0.5,
        bw,
        bh,
        Color::new(0.0, 0.0, 0.0, 0.55),
    );
    draw_text(
        label,
        cx - d.width * 0.5,
        cy + d.height * 0.5,
        font_size,
        Color::new(1.0, 1.0, 1.0, 0.9),
    );

    // Fill sweeps left-to-right under the label as the hold completes.
    let track_y = cy + bh * 0.5 + 6.0;
    draw_rectangle(
        cx - bw * 0.5,
        track_y,
        bw,
        3.0,
        Color::new(1.0, 1.0, 1.0, 0.25),
    );
    draw_rectangle(
        cx - bw * 0.5,
        track_y,
        bw * progress.clamp(0.0, 1.0),
        3.0,
        Color::new(1.0, 0.85, 0.35, 0.95),
    );
}

// Time-derived seed so each new game randomises its biome differently.
fn new_game_seed() -> u64 {
    (get_time() * 1_000_000.0) as u64
}

// None = no action, Some(None) = quit, Some(Some(s)) = transition
fn draw_startup_screen(selected: usize) -> Option<Option<AppScreen>> {
    let sw = screen_width();
    let sh = screen_height();
    let cx = sw * 0.5;

    draw_rectangle(0.0, 0.0, sw, sh, Color::from_rgba(15, 12, 8, 255));

    let title = "TOY HOVER BATTLE";
    let ts = 72.0;
    let tw = measure_text(title, None, ts as u16, 1.0).width;
    draw_text(
        title,
        cx - tw * 0.5,
        sh * 0.28,
        ts,
        Color::from_rgba(220, 190, 130, 255),
    );

    let labels = ["New Game", "Help", "Quit"];
    let btn_w = 260.0;
    let btn_h = 52.0;
    let gap = 18.0;
    let total = labels.len() as f32 * btn_h + (labels.len() - 1) as f32 * gap;
    let start_y = sh * 0.48;
    let mut result: Option<Option<AppScreen>> = None;

    for (i, label) in labels.iter().enumerate() {
        let by = start_y + i as f32 * (btn_h + gap);
        let rect = Rect::new(cx - btn_w * 0.5, by, btn_w, btn_h);
        let hover = rect.contains(mouse_position().into()) || selected == i;
        let bg = if hover {
            Color::from_rgba(180, 140, 60, 255)
        } else {
            Color::from_rgba(50, 40, 25, 255)
        };
        let border = Color::from_rgba(180, 140, 60, 255);
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, bg);
        draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, border);
        let lw = measure_text(label, None, 32, 1.0).width;
        draw_text(label, cx - lw * 0.5, by + btn_h * 0.65, 32.0, WHITE);
        if clicked(rect) {
            result = Some(match i {
                0 => Some(AppScreen::Playing),
                1 => Some(AppScreen::Help),
                _ => None, // Quit
            });
        }
    }

    // Keyboard hints
    let hint = "Enter / H / Esc";
    let hw = measure_text(hint, None, 20, 1.0).width;
    draw_text(
        hint,
        cx - hw * 0.5,
        start_y + total + gap * 2.0,
        20.0,
        Color::from_rgba(130, 110, 80, 255),
    );

    result
}

// Actions the match-setup screen can request from a mouse click.
enum SetupAction {
    None,
    Start,
    Back,
    Cycle(usize), // toggle player row `i`'s controller kind
    CycleSpeed,   // toggle the match-speed row
}

// Match-setup screen: P1/P2 rows cycling Human/Easy/Normal/Hard, a Speed row
// cycling Slow/Normal/Fast, plus Start and Back. `sel` is the keyboard focus
// (0,1 = player rows, 2 = speed row, 3 = Start, 4 = Back).
fn draw_setup_screen(sel: usize, kinds: [ControllerKind; 2], speed: GameSpeed) -> SetupAction {
    let sw = screen_width();
    let sh = screen_height();
    let cx = sw * 0.5;

    draw_rectangle(0.0, 0.0, sw, sh, Color::from_rgba(15, 12, 8, 255));

    let title = "MATCH SETUP";
    let ts = 64.0;
    let tw = measure_text(title, None, ts as u16, 1.0).width;
    draw_text(
        title,
        cx - tw * 0.5,
        sh * 0.2,
        ts,
        Color::from_rgba(220, 190, 130, 255),
    );

    let mut action = SetupAction::None;

    // Two player rows.
    let row_w = 460.0;
    let row_h = 56.0;
    let gap = 20.0;
    let start_y = sh * 0.32;
    let val_w = 180.0;
    let colors = [
        Color::from_rgba(100, 160, 255, 255),
        Color::from_rgba(255, 100, 70, 255),
    ];

    for i in 0..2 {
        let ry = start_y + i as f32 * (row_h + gap);
        let rx = cx - row_w * 0.5;
        let focused = sel == i;

        // Row label.
        let plabel = format!("Player {}", i + 1);
        draw_text(&plabel, rx, ry + row_h * 0.62, 32.0, colors[i]);

        // Value box (click to cycle) on the right of the row.
        let vx = rx + row_w - val_w;
        let vrect = Rect::new(vx, ry, val_w, row_h);
        let hover = vrect.contains(mouse_position().into()) || focused;
        let bg = if hover {
            Color::from_rgba(70, 55, 30, 255)
        } else {
            Color::from_rgba(40, 32, 20, 255)
        };
        draw_rectangle(vrect.x, vrect.y, vrect.w, vrect.h, bg);
        let border = if focused {
            Color::from_rgba(220, 180, 90, 255)
        } else {
            Color::from_rgba(120, 95, 50, 255)
        };
        draw_rectangle_lines(vrect.x, vrect.y, vrect.w, vrect.h, 2.0, border);

        // ‹ value ›
        let val = kinds[i].label();
        let vw = measure_text(val, None, 30, 1.0).width;
        draw_text(
            val,
            vx + val_w * 0.5 - vw * 0.5,
            ry + row_h * 0.62,
            30.0,
            WHITE,
        );
        draw_text(
            "<",
            vx + 12.0,
            ry + row_h * 0.62,
            30.0,
            Color::from_rgba(200, 170, 100, 255),
        );
        draw_text(
            ">",
            vx + val_w - 24.0,
            ry + row_h * 0.62,
            30.0,
            Color::from_rgba(200, 170, 100, 255),
        );

        if clicked(vrect) {
            action = SetupAction::Cycle(i);
        }
    }

    // Speed row, below the two player rows.
    {
        let ry = start_y + 2.0 * (row_h + gap);
        let rx = cx - row_w * 0.5;
        let focused = sel == 2;

        draw_text(
            "Speed",
            rx,
            ry + row_h * 0.62,
            32.0,
            Color::from_rgba(220, 190, 130, 255),
        );

        let vx = rx + row_w - val_w;
        let vrect = Rect::new(vx, ry, val_w, row_h);
        let hover = vrect.contains(mouse_position().into()) || focused;
        let bg = if hover {
            Color::from_rgba(70, 55, 30, 255)
        } else {
            Color::from_rgba(40, 32, 20, 255)
        };
        draw_rectangle(vrect.x, vrect.y, vrect.w, vrect.h, bg);
        let border = if focused {
            Color::from_rgba(220, 180, 90, 255)
        } else {
            Color::from_rgba(120, 95, 50, 255)
        };
        draw_rectangle_lines(vrect.x, vrect.y, vrect.w, vrect.h, 2.0, border);

        let val = speed.label();
        let vw = measure_text(val, None, 30, 1.0).width;
        draw_text(
            val,
            vx + val_w * 0.5 - vw * 0.5,
            ry + row_h * 0.62,
            30.0,
            WHITE,
        );
        draw_text(
            "<",
            vx + 12.0,
            ry + row_h * 0.62,
            30.0,
            Color::from_rgba(200, 170, 100, 255),
        );
        draw_text(
            ">",
            vx + val_w - 24.0,
            ry + row_h * 0.62,
            30.0,
            Color::from_rgba(200, 170, 100, 255),
        );

        if clicked(vrect) {
            action = SetupAction::CycleSpeed;
        }
    }

    // Start / Back buttons.
    let btn_w = 200.0;
    let btn_h = 52.0;
    let by = start_y + 3.0 * (row_h + gap) + 30.0;
    let labels = ["Start", "Back"];
    let total = labels.len() as f32 * btn_w + (labels.len() - 1) as f32 * 24.0;
    let bx0 = cx - total * 0.5;
    for (i, label) in labels.iter().enumerate() {
        let bx = bx0 + i as f32 * (btn_w + 24.0);
        let rect = Rect::new(bx, by, btn_w, btn_h);
        let focused = sel == 3 + i;
        let hover = rect.contains(mouse_position().into()) || focused;
        let bg = if hover {
            Color::from_rgba(180, 140, 60, 255)
        } else {
            Color::from_rgba(50, 40, 25, 255)
        };
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, bg);
        draw_rectangle_lines(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            2.0,
            Color::from_rgba(180, 140, 60, 255),
        );
        let lw = measure_text(label, None, 32, 1.0).width;
        draw_text(
            label,
            bx + btn_w * 0.5 - lw * 0.5,
            by + btn_h * 0.65,
            32.0,
            WHITE,
        );
        if clicked(rect) {
            action = if i == 0 {
                SetupAction::Start
            } else {
                SetupAction::Back
            };
        }
    }

    let hint = "↑↓ select   ←→ change   Enter / Esc";
    let hw = measure_text(hint, None, 20, 1.0).width;
    draw_text(
        hint,
        cx - hw * 0.5,
        by + btn_h + 36.0,
        20.0,
        Color::from_rgba(130, 110, 80, 255),
    );

    action
}

fn draw_help_screen() -> bool {
    let sw = screen_width();
    let sh = screen_height();
    let cx = sw * 0.5;

    draw_rectangle(0.0, 0.0, sw, sh, Color::from_rgba(15, 12, 8, 255));

    let title = "CONTROLS";
    let ts = 56.0;
    let tw = measure_text(title, None, ts as u16, 1.0).width;
    draw_text(
        title,
        cx - tw * 0.5,
        sh * 0.12,
        ts,
        Color::from_rgba(220, 190, 130, 255),
    );

    let col_w = sw * 0.38;
    let col1_x = cx - col_w - 20.0;
    let col2_x = cx + 20.0;
    let top_y = sh * 0.22;
    let line_h = 34.0;

    let p1_header = "Player 1";
    draw_text(
        p1_header,
        col1_x,
        top_y,
        36.0,
        Color::from_rgba(100, 160, 255, 255),
    );

    let p2_header = "Player 2";
    draw_text(
        p2_header,
        col2_x,
        top_y,
        36.0,
        Color::from_rgba(255, 100, 70, 255),
    );

    let rows: &[(&str, &str, &str)] = &[
        ("Throttle / Brake", "W / S", "Arrow Up / Down"),
        ("Turn", "A / D", "Arrow Left / Right"),
        ("Strafe", "R / T", ", / ."),
        ("Fire", "Left Shift", "Right Shift / /"),
        ("Rocket", "Q", "Enter"),
    ];

    for (i, (action, p1, p2)) in rows.iter().enumerate() {
        let y = top_y + line_h + i as f32 * line_h;
        let dim = Color::from_rgba(160, 145, 120, 255);
        let bright = WHITE;
        draw_text(action, col1_x, y, 24.0, dim);
        draw_text(p1, col1_x + col_w * 0.45, y, 24.0, bright);
        draw_text(p2, col2_x + col_w * 0.45, y, 24.0, bright);
    }

    // Quitting is shared between both players rather than per-column, and a
    // hold isn't something anyone discovers by accident — so it gets a line.
    let quit_hint = "Quit a match:  hold Esc / Start,  or Ctrl+C";
    let qw = measure_text(quit_hint, None, 22, 1.0).width;
    draw_text(
        quit_hint,
        cx - qw * 0.5,
        top_y + line_h * (rows.len() as f32 + 2.0),
        22.0,
        Color::from_rgba(160, 145, 120, 255),
    );

    let btn_label = "Back";
    let btn_w = 180.0;
    let btn_h = 48.0;
    let bx = cx - btn_w * 0.5;
    let by = sh * 0.82;
    let rect = Rect::new(bx, by, btn_w, btn_h);
    let hover = rect.contains(mouse_position().into());
    let bg = if hover {
        Color::from_rgba(180, 140, 60, 255)
    } else {
        Color::from_rgba(50, 40, 25, 255)
    };
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, bg);
    draw_rectangle_lines(
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        2.0,
        Color::from_rgba(180, 140, 60, 255),
    );
    let lw = measure_text(btn_label, None, 30, 1.0).width;
    draw_text(btn_label, cx - lw * 0.5, by + btn_h * 0.65, 30.0, WHITE);

    let hint = "Esc / Backspace";
    let hw = measure_text(hint, None, 20, 1.0).width;
    draw_text(
        hint,
        cx - hw * 0.5,
        by + btn_h + 24.0,
        20.0,
        Color::from_rgba(130, 110, 80, 255),
    );

    clicked(rect)
}

const POST_VERTEX_SHADER: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;

varying lowp vec2 uv;
varying lowp vec4 color;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    color = color0 / 255.0;
    uv = texcoord;
}
"#;

const DEATH_FADE_FRAGMENT_SHADER: &str = r#"#version 100
precision lowp float;

varying lowp vec2 uv;
varying lowp vec4 color;

uniform sampler2D Texture;
uniform lowp float DeathFade;

void main() {
    vec4 tex = texture2D(Texture, uv) * color;
    float gray = dot(tex.rgb, vec3(0.299, 0.587, 0.114));
    vec3 bw = vec3(gray);
    gl_FragColor = vec4(mix(tex.rgb, bw, DeathFade), tex.a);
}
"#;

const DEATH_FADE_METAL_SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Uniforms
{
    float4x4 Model;
    float4x4 Projection;
    float DeathFade;
};

struct Vertex
{
    float3 position [[attribute(0)]];
    float2 texcoord [[attribute(1)]];
    float4 color0 [[attribute(2)]];
};

struct RasterizerData
{
    float4 position [[position]];
    float4 color [[user(locn0)]];
    float2 uv [[user(locn1)]];
};

vertex RasterizerData vertexShader(Vertex v [[stage_in]], constant Uniforms& uniforms [[buffer(0)]])
{
    RasterizerData out;

    out.position = uniforms.Model * uniforms.Projection * float4(v.position, 1);
    out.color = v.color0 / 255.0;
    out.uv = v.texcoord;

    return out;
}

fragment float4 fragmentShader(
    RasterizerData in [[stage_in]],
    texture2d<float> tex [[texture(0)]],
    sampler texSmplr [[sampler(0)]],
    constant Uniforms& uniforms [[buffer(0)]]
)
{
    float4 sample = in.color * tex.sample(texSmplr, in.uv);
    float gray = dot(sample.rgb, float3(0.299, 0.587, 0.114));
    float3 bw = float3(gray);
    return float4(mix(sample.rgb, bw, uniforms.DeathFade), sample.a);
}
"#;

fn window_conf() -> Conf {
    Conf {
        window_title: "Toy Hover Battle".into(),
        window_width: 1920,
        window_height: 1080,
        window_resizable: true,
        high_dpi: false,
        sample_count: 1,
        icon: Some(Icon {
            small: *include_bytes!("../assets/icon_16.rgba"),
            medium: *include_bytes!("../assets/icon_32.rgba"),
            big: *include_bytes!("../assets/icon_64.rgba"),
        }),
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    // Two half-width targets for split screen, one full-width target for the
    // single-viewpoint layout. All three live for the whole run; which pair of
    // cameras is used is decided per frame from the match's controller kinds.
    let rt1 = render_target(VW, VH);
    let rt2 = render_target(VW, VH);
    let rt_full = render_target(VW_FULL, VH);
    rt1.texture.set_filter(FilterMode::Nearest);
    rt2.texture.set_filter(FilterMode::Nearest);
    rt_full.texture.set_filter(FilterMode::Nearest);
    let ship_sprites = assets::ShipSprites::load();
    let rock_sprites = assets::RockSprites::load();
    let explosion_sprites = assets::ExplosionSprites::load();
    let rocket_sprites = assets::RocketSprites::load();
    let death_fade_material = load_material(
        unsafe {
            let gl = get_internal_gl();
            match gl.quad_context.info().backend {
                Backend::OpenGl => ShaderSource::Glsl {
                    vertex: POST_VERTEX_SHADER,
                    fragment: DEATH_FADE_FRAGMENT_SHADER,
                },
                Backend::Metal => ShaderSource::Msl {
                    program: DEATH_FADE_METAL_SHADER,
                },
            }
        },
        MaterialParams {
            uniforms: vec![UniformDesc::new("DeathFade", UniformType::Float1)],
            ..Default::default()
        },
    )
    .expect("death fade material");

    let mut sfx = Sfx::load().await;
    let bg = Background::new();
    let mut biome = BiomeParams::random(new_game_seed());

    let display = Rect::new(0., 0., VW as f32, VH as f32);
    let mut cam1 = Camera2D::from_display_rect(display);
    let mut cam2 = Camera2D::from_display_rect(display);
    cam1.render_target = Some(rt1.clone());
    cam2.render_target = Some(rt2.clone());

    let mut cam_full = Camera2D::from_display_rect(Rect::new(0., 0., VW_FULL as f32, VH as f32));
    cam_full.render_target = Some(rt_full.clone());

    let mut state = GameState::new(
        [ControllerKind::Human, ControllerKind::Human],
        GameSpeed::Normal,
    );
    let mut pads = pads::Pads::new();
    let mut screen = AppScreen::Startup;
    let mut menu_sel: usize = 0; // keyboard-selected button on startup screen
    let mut exit_hold = HoldToConfirm::default(); // quit-chord hold for the live match

    // Match-setup selections — default Human P1 vs Normal AI P2, focus on Start.
    let mut setup_kinds: [ControllerKind; 2] = [
        ControllerKind::Human,
        ControllerKind::Ai(AiDifficulty::Normal),
    ];
    let mut setup_speed: GameSpeed = GameSpeed::Normal;
    let mut setup_sel: usize = 3;

    'main: loop {
        // miniquad's frame timer is wall-clock, not monotonic: an NTP step or
        // clock adjustment can make it briefly negative, which corrupts every
        // dt-scaled integration downstream (turret traverse, movement, timers)
        // and panics the fixed symmetric `.clamp(-rate * dt, rate * dt)` calls.
        let dt = get_frame_time().max(0.0);
        pads.update();
        // Menu navigation reuses ship movement's own buttons (dpad, A/B),
        // computed every frame regardless of screen so edge-tracking stays in
        // sync — otherwise a button held across a screen transition could
        // read as a fresh press when the menu that finally checks it starts.
        let menu = pads.menu_input();

        match screen {
            AppScreen::Startup => {
                set_default_camera();

                // Keyboard/controller navigation — handle and skip drawing on transition
                if is_key_pressed(KeyCode::Up) || menu.up {
                    let prev = menu_sel;
                    menu_sel = menu_sel.saturating_sub(1);
                    if menu_sel != prev {
                        sfx.play(SfxEvent::MenuMove);
                    }
                }
                if is_key_pressed(KeyCode::Down) || menu.down {
                    let prev = menu_sel;
                    menu_sel = (menu_sel + 1).min(2);
                    if menu_sel != prev {
                        sfx.play(SfxEvent::MenuMove);
                    }
                }
                if is_key_pressed(KeyCode::Escape) || menu.back {
                    sfx.play(SfxEvent::MenuConfirm);
                    break 'main;
                }
                if is_key_pressed(KeyCode::H) {
                    sfx.play(SfxEvent::MenuConfirm);
                    screen = AppScreen::Help;
                    next_frame().await;
                    continue;
                }
                if is_key_pressed(KeyCode::Enter) || menu.confirm {
                    sfx.play(SfxEvent::MenuConfirm);
                    match menu_sel {
                        0 => {
                            setup_sel = 3;
                            screen = AppScreen::Setup;
                        }
                        1 => {
                            screen = AppScreen::Help;
                        }
                        _ => break 'main,
                    }
                    next_frame().await;
                    continue;
                }

                match draw_startup_screen(menu_sel) {
                    None => {} // no action this frame
                    Some(None) => {
                        sfx.play(SfxEvent::MenuConfirm);
                        break 'main;
                    }
                    Some(Some(AppScreen::Playing)) => {
                        // "New Game" → match setup (not straight into a game).
                        sfx.play(SfxEvent::MenuConfirm);
                        setup_sel = 3;
                        screen = AppScreen::Setup;
                    }
                    Some(Some(next)) => {
                        sfx.play(SfxEvent::MenuConfirm);
                        screen = next;
                    }
                }
            }

            AppScreen::Setup => {
                set_default_camera();

                let mut start = false;
                let mut back = false;

                if is_key_pressed(KeyCode::Up) || menu.up {
                    let prev = setup_sel;
                    setup_sel = setup_sel.saturating_sub(1);
                    if setup_sel != prev {
                        sfx.play(SfxEvent::MenuMove);
                    }
                }
                if is_key_pressed(KeyCode::Down) || menu.down {
                    let prev = setup_sel;
                    setup_sel = (setup_sel + 1).min(4);
                    if setup_sel != prev {
                        sfx.play(SfxEvent::MenuMove);
                    }
                }
                if setup_sel < 2 {
                    if is_key_pressed(KeyCode::Right) || menu.right {
                        setup_kinds[setup_sel] = setup_kinds[setup_sel].next();
                        sfx.play(SfxEvent::MenuMove);
                    }
                    if is_key_pressed(KeyCode::Left) || menu.left {
                        setup_kinds[setup_sel] = setup_kinds[setup_sel].prev();
                        sfx.play(SfxEvent::MenuMove);
                    }
                } else if setup_sel == 2 {
                    if is_key_pressed(KeyCode::Right) || menu.right {
                        setup_speed = setup_speed.next();
                        sfx.play(SfxEvent::MenuMove);
                    }
                    if is_key_pressed(KeyCode::Left) || menu.left {
                        setup_speed = setup_speed.prev();
                        sfx.play(SfxEvent::MenuMove);
                    }
                }
                if is_key_pressed(KeyCode::Enter) || menu.confirm {
                    match setup_sel {
                        0 | 1 => {
                            setup_kinds[setup_sel] = setup_kinds[setup_sel].next();
                            sfx.play(SfxEvent::MenuMove);
                        }
                        2 => {
                            setup_speed = setup_speed.next();
                            sfx.play(SfxEvent::MenuMove);
                        }
                        3 => start = true,
                        _ => back = true,
                    }
                }
                if is_key_pressed(KeyCode::Escape)
                    || is_key_pressed(KeyCode::Backspace)
                    || menu.back
                {
                    back = true;
                }

                match draw_setup_screen(setup_sel, setup_kinds, setup_speed) {
                    SetupAction::None => {}
                    SetupAction::Start => start = true,
                    SetupAction::Back => back = true,
                    SetupAction::Cycle(i) => {
                        setup_kinds[i] = setup_kinds[i].next();
                        sfx.play(SfxEvent::MenuMove);
                    }
                    SetupAction::CycleSpeed => {
                        setup_speed = setup_speed.next();
                        sfx.play(SfxEvent::MenuMove);
                    }
                }

                if start {
                    sfx.play(SfxEvent::MenuConfirm);
                    state = GameState::new(setup_kinds, setup_speed);
                    biome = BiomeParams::random(new_game_seed());
                    screen = AppScreen::Playing;
                    // Start the match with a clean hold timer: a key still down
                    // from the menu must not carry progress into the match.
                    exit_hold.reset();
                    // Same idea for a controller's rocket button: confirming
                    // "Start" with it must not also read as the first frame's
                    // rocket-fire press.
                    pads.reset_match_input();
                    next_frame().await;
                    continue;
                }
                if back {
                    sfx.play(SfxEvent::MenuConfirm);
                    screen = AppScreen::Startup;
                }
            }

            AppScreen::Help => {
                set_default_camera();

                if is_key_pressed(KeyCode::Escape)
                    || is_key_pressed(KeyCode::Backspace)
                    || menu.confirm
                    || menu.back
                {
                    sfx.play(SfxEvent::MenuConfirm);
                    screen = AppScreen::Startup;
                }
                if draw_help_screen() {
                    sfx.play(SfxEvent::MenuConfirm);
                    screen = AppScreen::Startup;
                }
            }

            AppScreen::Playing => {
                state.update(dt, &mut pads);
                for event in state.drain_sfx_events() {
                    sfx.play(event);
                }

                // One human in the match → a single full-window viewport for
                // them; otherwise the side-by-side split.
                let layout = state.layout();
                let view_size = layout.view_size();

                let t = get_time() as f32;
                let shake_offset = |shake: f32, phase: f32| -> Vec2 {
                    if shake <= 0.0 {
                        return Vec2::ZERO;
                    }
                    vec2((t * 97.3 + phase).sin(), (t * 83.7 + phase).cos()) * shake * 10.0
                };
                let cam_targets = [
                    iso::world_to_screen(state.players[0].pos)
                        + shake_offset(state.players[0].camera_shake, 0.0),
                    iso::world_to_screen(state.players[1].pos)
                        + shake_offset(state.players[1].camera_shake, 1.57),
                ];

                // Live ships perturb the background flow under both viewports.
                // `*_dirs` carry each vehicle's exhaust direction (−heading) so the
                // desert sand wake trails out behind its engines.
                let ships: Vec<Vec3> = state
                    .players
                    .iter()
                    .filter(|p| !p.is_dead())
                    .map(|p| vec3(p.pos.x, p.pos.y, p.radius))
                    .collect();
                let ship_dirs: Vec<Vec2> = state
                    .players
                    .iter()
                    .filter(|p| !p.is_dead())
                    .map(|p| p.exhaust_dir)
                    .collect();
                // Signed throttle: + forward → trail behind; − braking → shorter
                // trail in front; ~0 → short round puff.
                let ship_thrust: Vec<f32> = state
                    .players
                    .iter()
                    .filter(|p| !p.is_dead())
                    .map(|p| p.thrust.clamp(-1.0, 1.0))
                    .collect();
                let rockets: Vec<Vec3> = state
                    .rockets
                    .iter()
                    .map(|r| vec3(r.pos.x, r.pos.y, bullet::Rocket::RADIUS))
                    .collect();
                let rocket_dirs: Vec<Vec2> = state
                    .rockets
                    .iter()
                    .map(|r| -vec2(r.angle.cos(), r.angle.sin()))
                    .collect();

                let draw_start = get_time();

                match layout {
                    Layout::Single { viewer } => {
                        cam_full.target = cam_targets[viewer];
                        set_camera(&cam_full);
                        bg.draw(
                            &biome,
                            BackgroundView {
                                world_pos: iso::screen_to_world(cam_full.target),
                                target: cam_full.target,
                                size: view_size,
                            },
                            &state.world,
                            BackgroundObstacles {
                                ships: &ships,
                                ship_dirs: &ship_dirs,
                                ship_thrust: &ship_thrust,
                                rockets: &rockets,
                                rocket_dirs: &rocket_dirs,
                            },
                        );
                        state.draw_world(
                            viewer,
                            view_size,
                            &ship_sprites,
                            &rock_sprites,
                            &explosion_sprites,
                            &rocket_sprites,
                        );
                    }
                    Layout::Split => {
                        cam1.target = cam_targets[0];
                        cam2.target = cam_targets[1];

                        set_camera(&cam1);
                        bg.draw(
                            &biome,
                            BackgroundView {
                                world_pos: iso::screen_to_world(cam1.target),
                                target: cam1.target,
                                size: view_size,
                            },
                            &state.world,
                            BackgroundObstacles {
                                ships: &ships,
                                ship_dirs: &ship_dirs,
                                ship_thrust: &ship_thrust,
                                rockets: &rockets,
                                rocket_dirs: &rocket_dirs,
                            },
                        );
                        state.draw_world(
                            0,
                            view_size,
                            &ship_sprites,
                            &rock_sprites,
                            &explosion_sprites,
                            &rocket_sprites,
                        );

                        set_camera(&cam2);
                        bg.draw(
                            &biome,
                            BackgroundView {
                                world_pos: iso::screen_to_world(cam2.target),
                                target: cam2.target,
                                size: view_size,
                            },
                            &state.world,
                            BackgroundObstacles {
                                ships: &ships,
                                ship_dirs: &ship_dirs,
                                ship_thrust: &ship_thrust,
                                rockets: &rockets,
                                rocket_dirs: &rocket_dirs,
                            },
                        );
                        state.draw_world(
                            1,
                            view_size,
                            &ship_sprites,
                            &rock_sprites,
                            &explosion_sprites,
                            &rocket_sprites,
                        );
                    }
                }

                set_default_camera();
                clear_background(BLACK);
                let sw = screen_width();
                let hw = sw * 0.5;
                let h = screen_height();

                // Render targets are stored y-flipped relative to screen-space;
                // a negative source height flips them on the way out.
                gl_use_material(&death_fade_material);
                match layout {
                    Layout::Single { viewer } => {
                        let params = DrawTextureParams {
                            dest_size: Some(vec2(sw, h)),
                            source: Some(Rect::new(0.0, VH as f32, VW_FULL as f32, -(VH as f32))),
                            ..Default::default()
                        };
                        death_fade_material.set_uniform("DeathFade", state.death_view_fade(viewer));
                        draw_texture_ex(&rt_full.texture, 0.0, 0.0, WHITE, params);
                    }
                    Layout::Split => {
                        let src = Rect::new(0.0, VH as f32, VW as f32, -(VH as f32));
                        let params_left = DrawTextureParams {
                            dest_size: Some(vec2(hw, h)),
                            source: Some(src),
                            ..Default::default()
                        };
                        let params_right = params_left.clone();
                        death_fade_material.set_uniform("DeathFade", state.death_view_fade(0));
                        draw_texture_ex(&rt1.texture, 0.0, 0.0, WHITE, params_left);
                        death_fade_material.set_uniform("DeathFade", state.death_view_fade(1));
                        draw_texture_ex(&rt2.texture, hw, 0.0, WHITE, params_right);
                    }
                }
                gl_use_default_material();

                if layout == Layout::Split {
                    draw_rectangle(hw - 1.0, 0.0, 2.0, h, BLACK);
                }
                state.draw_hud();
                state.draw_ms = ((get_time() - draw_start) * 1000.0) as f32;

                // Two ways out of a match, differing in how easily they can be
                // hit by accident. Escape is one key and sits next to nothing,
                // so it must be *held* — a match is shared state between two
                // players and a stray tap shouldn't end it for both. Ctrl+C is
                // already a deliberate two-key chord, so it needs no such
                // guard and quits on the spot. A controller's Start button
                // gets the same hold requirement as Escape, for the same
                // reason.
                let quit_now = exit_now_pressed();
                if quit_now
                    || exit_hold.tick(exit_hold_down() || pads.start_held(), dt, EXIT_HOLD_TIME)
                {
                    sfx.play(SfxEvent::MenuConfirm);
                    screen = AppScreen::Startup;
                    menu_sel = 0;
                    exit_hold.reset();
                } else if exit_hold.in_progress() {
                    draw_exit_hold_prompt(exit_hold.progress(EXIT_HOLD_TIME));
                }
            }
        }

        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    /// Hold `secs` worth of frames; returns how many times the gesture fired.
    /// Rounds rather than truncates: `secs / DT` lands a hair under the whole
    /// frame count in f32 (0.9 - 2/60 gives 51.999996), and truncating there
    /// silently drops a frame and shifts every threshold assertion below.
    fn hold(h: &mut HoldToConfirm, secs: f32) -> i32 {
        let mut fired = 0;
        for _ in 0..(secs / DT).round() as i32 {
            if h.tick(true, DT, EXIT_HOLD_TIME) {
                fired += 1;
            }
        }
        fired
    }

    /// The whole point of the change: a stray tap must not abandon the match.
    #[test]
    fn a_tap_never_fires() {
        let mut h = HoldToConfirm::default();
        for _ in 0..20 {
            assert_eq!(hold(&mut h, EXIT_HOLD_TIME * 0.5), 0);
            assert!(!h.tick(false, DT, EXIT_HOLD_TIME), "fired on release");
        }
    }

    /// Repeated near-misses must not accumulate — progress is discarded on
    /// release, not drained, so tapping can never inch toward the threshold.
    #[test]
    fn released_progress_is_discarded_not_drained() {
        let mut h = HoldToConfirm::default();
        hold(&mut h, EXIT_HOLD_TIME * 0.9);
        assert!(h.progress(EXIT_HOLD_TIME) > 0.8);
        h.tick(false, DT, EXIT_HOLD_TIME);
        assert_eq!(h.progress(EXIT_HOLD_TIME), 0.0, "progress survived release");
    }

    #[test]
    fn a_sustained_hold_fires_once_at_the_threshold() {
        let mut h = HoldToConfirm::default();
        // Just short of the threshold: nothing yet.
        assert_eq!(hold(&mut h, EXIT_HOLD_TIME - 2.0 * DT), 0);
        // Crossing it fires exactly once, not every frame thereafter.
        assert_eq!(hold(&mut h, 3.0 * DT), 1);
    }

    #[test]
    fn progress_ramps_and_reset_clears_it() {
        let mut h = HoldToConfirm::default();
        assert!(!h.in_progress());
        hold(&mut h, EXIT_HOLD_TIME * 0.5);
        assert!(h.in_progress());
        let p = h.progress(EXIT_HOLD_TIME);
        assert!(
            (0.4..0.6).contains(&p),
            "progress {p} should track the hold"
        );
        h.reset();
        assert!(!h.in_progress());
        assert_eq!(h.progress(EXIT_HOLD_TIME), 0.0);
    }

    /// A frame-time spike must not overshoot into a runaway value the prompt
    /// would render past the end of its track.
    #[test]
    fn progress_is_clamped_on_a_long_frame() {
        let mut h = HoldToConfirm::default();
        h.tick(true, EXIT_HOLD_TIME * 0.99, EXIT_HOLD_TIME);
        assert!(h.progress(EXIT_HOLD_TIME) <= 1.0);
    }
}
