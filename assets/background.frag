#version 100
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying lowp vec2 uv;

uniform vec2 u_resolution;
uniform float u_time;
uniform int u_seed;
uniform vec2 u_cam_origin;
uniform float u_pixels_per_unit;
uniform vec2 u_wind_dir;
uniform float u_dune_stretch;
uniform float u_warp_amp;
uniform vec3 u_sun_dir;
uniform vec3 u_palette_a;
uniform vec3 u_palette_b;
uniform vec3 u_palette_c;
uniform int u_octaves;
uniform vec2 u_terrain_offset;
uniform float u_flow_speed;
uniform float u_ridge_exponent;

#define TILE_W 64.0
#define TILE_H 32.0
#define TAU    6.28318530

#define MAX_ROCKS 64          // keep in sync with background.rs

#define ROCK_R_SCALE 1.4      // visual radius ≈ collision radius × this

#define SHIP_R_SCALE 1.8

#define MAX_BG_ROCKETS   4
#define ROCKET_R_SCALE   3.0  // visual radius / physical radius (rockets are tiny, give them presence)

// Desert blowing sand — additive airborne grain overlay behind engines.
#define DUST_SHIP_PLUME_IDLE 0.35 // ship wake length when coasting (short round puff)
#define DUST_SHIP_PLUME_FULL 1.8  // ship wake length at full forward thrust
#define DUST_SHIP_PLUME_BRAKE 1.2 // ship wake length at full braking (shorter, in front)
#define DUST_ROCKET_PLUME  3.0  // rocket exhaust plume length
#define DUST_WAKE_WIDTH    0.8 // wake half-width (× radius) — slim central stream
#define DUST_GRAIN_FREQ    0.8  // grain spatial frequency (lower = coarser, less shimmer)
#define DUST_SPEED         2.8  // grain drift rate (tiles/sec)
#define DUST_BRIGHTNESS    2.8  // additive intensity of lit airborne grains

uniform vec3 u_rocks[MAX_ROCKS]; // xy = world pos, z = collision radius
uniform int u_rock_count;
uniform vec3 u_ships[2]; // xy = world pos, z = collision radius
uniform vec2 u_ship_dir[2]; // unit exhaust direction (points behind the ship)
uniform float u_ship_thrust[2]; // signed throttle [-1,1]: + behind, − in front
uniform int u_ship_count;
uniform vec3 u_rockets[MAX_BG_ROCKETS]; // xy = world pos, z = collision radius
uniform vec2 u_rocket_dir[MAX_BG_ROCKETS]; // unit exhaust direction (points behind the rocket)
uniform int u_rocket_count;

// ── World-position reconstruction ─────────────────────────────────────────────
//
// gl_FragCoord.y is OpenGL-convention (0 = bottom of render target).
// macroquad's camera uses y-down convention and the render target is blitted
// with flip_y, so we flip y here to align with the iso projection.

vec2 frag_to_world(vec2 fc) {
    float sx = (fc.x - 0.5 * u_resolution.x) / u_pixels_per_unit;
    float sy = (0.5 * u_resolution.y - fc.y) / u_pixels_per_unit;
    float wx = sx / TILE_W + sy / TILE_H;
    float wy = sy / TILE_H - sx / TILE_W;
    return vec2(wx, wy) + u_cam_origin;
}

// ── Gradient noise ────────────────────────────────────────────────────────────

float hash(vec2 p) {
    p = fract(p * vec2(127.1, 311.7));
    p += dot(p, p + 19.19);
    return fract(p.x * p.y);
}

// Gradient noise, range ≈ [−0.7, 0.7]
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);

    float ha = hash(i);
    float hb = hash(i + vec2(1.0, 0.0));
    float hc = hash(i + vec2(0.0, 1.0));
    float hd = hash(i + vec2(1.0, 1.0));

    float ga = dot(vec2(cos(ha * TAU), sin(ha * TAU)), f);
    float gb = dot(vec2(cos(hb * TAU), sin(hb * TAU)), f - vec2(1.0, 0.0));
    float gc = dot(vec2(cos(hc * TAU), sin(hc * TAU)), f - vec2(0.0, 1.0));
    float gd = dot(vec2(cos(hd * TAU), sin(hd * TAU)), f - vec2(1.0, 1.0));

    return mix(mix(ga, gb, u.x), mix(gc, gd, u.x), u.y);
}

// Fractional Brownian Motion — range ≈ [−0.7, 0.7]
float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8); // slight rotation breaks axis alignment
    for (int i = 0; i < 8; i++) {
        // Keep this loop statically bounded: older NVIDIA drivers can evaluate
        // work past a uniform-controlled break and contaminate the fragment.
        if (i < u_octaves) {
            v += a * vnoise(p);
        }
        p = rot * p * 2.0 + vec2(100.0);
        a *= 0.5;
    }
    return v;
}

// ── Desert heightfield ────────────────────────────────────────────────────────

float desert_h(vec2 wp) {
    wp += u_terrain_offset;
    vec2 p = wp * 0.09;
    p -= normalize(u_wind_dir) * u_time * 0.016 * u_flow_speed;

    vec2 w = normalize(u_wind_dir);
    vec2 perp = vec2(-w.y, w.x);
    // Anisotropy: elongate perpendicular to wind → long ridge dunes
    vec2 ap = dot(p, w) * w + (dot(p, perp) / u_dune_stretch) * perp;

    // Domain warp
    vec2 q = vec2(fbm(ap), fbm(ap + vec2(5.2, 1.3)));
    vec2 warped = ap + u_warp_amp * q;

    // Ridged FBM with sharpened crests: pow > 1 narrows the peaks, flattens troughs.
    // FBM can overshoot its approximate range, so clamp before a fractional pow.
    float ridge = pow(max(0.0, 1.0 - abs(fbm(warped))), u_ridge_exponent);

    // Fine sand ripples — same anisotropy at ~5× the dune frequency
    vec2 rp = wp * 0.48;
    rp -= w * u_time * 0.045 * u_flow_speed;
    rp = dot(rp, w) * w + (dot(rp, perp) / u_dune_stretch) * perp;
    float ripple = pow(1.0 - abs(vnoise(rp)), 3.0);

    return ridge + 0.12 * ripple;
}

// Fine sand micro-relief — high-frequency bumps (a few pixels across) layered on
// top of the macro dunes. Without this the surface is all smooth low-frequency
// gradients and reads like flowing liquid; the grain makes it gritty sand.
#define GRAIN_RELIEF 0.02 // micro-bump normal-tilt strength
#define GRAIN_ALBEDO 0.06  // fine bright/dark speckle on the albedo
float micro_relief(vec2 wp) {
    return vnoise(wp * 5.5) + 0.5 * vnoise(wp * 9.0 + vec2(17.3, 4.1));
}

vec3 shade_desert(vec2 wp) {
    float eps = 0.3; // tighter finite-difference step → sharper normal detail
    float h0 = desert_h(wp);
    float hx = desert_h(wp + vec2(eps, 0.0));
    float hy = desert_h(wp + vec2(0.0, eps));

    vec3 N = normalize(vec3(-(hx - h0) / eps, -(hy - h0) / eps, 0.9));

    // Tilt the macro normal by the gradient of the fine grain so the sand catches
    // light at grain scale. Small finite-diff step matched to the grain frequency.
    float me = 0.05;
    float m0 = micro_relief(wp);
    float mx = micro_relief(wp + vec2(me, 0.0));
    float my = micro_relief(wp + vec2(0.0, me));
    vec2 mgrad = vec2(mx - m0, my - m0) / me;
    N = normalize(N + vec3(-mgrad * GRAIN_RELIEF, 0.0));

    vec3 L = normalize(u_sun_dir);
    vec3 V = vec3(0.0, 0.0, 1.0);

    // Three-level height colour ramp:
    //   deep trough → reddish-brown shadow sand → bright lit crest
    // High ridge exponents deliberately concentrate terrain near zero. A
    // square-root response retains sharp crests while keeping low dune relief
    // out of the uniformly dark trough range.
    float hn = sqrt(clamp(h0, 0.0, 1.0));
    vec3 trough = vec3(0.52, 0.33, 0.16);
    vec3 base = mix(trough, u_palette_a, smoothstep(0.0, 0.35, hn));
    base = mix(base, u_palette_b, smoothstep(0.55, 1.0, hn));

    // Half-Lambert wrap diffuse
    float diff = pow(clamp(0.5 + 0.5 * dot(N, L), 0.0, 1.0), 1.5);
    // Stronger AO: troughs are substantially darker than crests
    float ao = 0.45 + 0.55 * pow(hn, 1.2);
    // Rim glow on back-lit silhouette edges
    float rim = pow(clamp(1.0 - dot(N, L), 0.0, 1.0), 3.0)
            * clamp(0.3 - dot(N, L), 0.0, 1.0);
    // Broad sandy specular sheen (sand grain retroreflection)
    vec3 H = normalize(L + V);
    float spec = pow(clamp(dot(N, H), 0.0, 1.0), 10.0) * 0.15;

    vec3 col = base * diff * ao;
    col += u_palette_c * rim * 0.45;
    col += u_palette_b * spec;
    // Fine speckle: scatter tiny bright/dark grains across the albedo for grit.
    col *= 1.0 + clamp(m0, -1.0, 1.0) * GRAIN_ALBEDO;
    return col;
}

// ── Desert blowing sand ────────────────────────────────────────────────────────
//
// Grain speckle drifting *outward* along the exhaust direction `dir` over time,
// so the wake visibly flows off the back of the vehicle. Sampled in the
// engine-relative frame `d` (bounded to a few tiles), so a turning ship only
// nudges the coordinates slightly — no large jumps. The time scroll is a scalar
// offset on the along-axis (it never rotates), and the frequency is low, so the
// grains drift smoothly without aliasing/shimmer.
float wake_grains(vec2 d, vec2 dir) {
    vec2 perp = vec2(-dir.y, dir.x);
    float along = dot(d, dir) - u_time * DUST_SPEED; // scroll outward, bounded coords
    float cross = dot(d, perp);
    vec2 c = vec2(along, cross) * DUST_GRAIN_FREQ;
    float g = vnoise(c) + 0.5 * vnoise(c * 2.0 + vec2(19.0, 7.0));
    return smoothstep(-0.1, 0.6, g); // soft, coarse grains → no shimmer
}

// Engine-wake lobe: a slim plume concentrated behind a vehicle, extending along
// its exhaust direction `dir`. The lobe origin sits at the engine (just behind
// the hull centre), so dust trails out of the central engine, not the whole body.
// Returns lobe shape × outward-flowing grains.
float engine_wake(vec2 wp, vec2 center, vec2 dir, float R, float plume, float falloff) {
    vec2 perp = vec2(-dir.y, dir.x);
    vec2 d = wp - (center + dir * R * 0.6); // shift origin to the engine
    float along = dot(d, dir); // + = behind the engine
    float cross = dot(d, perp);
    float aw = along > 0.0 ? along / (R * plume) : along / (R * 0.45); // long behind, short ahead
    float cw = cross / (R * DUST_WAKE_WIDTH); // slim central stream
    float lobe = exp(-(aw * aw + cw * cw) * falloff);
    return lobe * wake_grains(d, dir);
}

// GLSL ES 1.00 permits uniform arrays, but dynamic indexing of them is not
// reliably implemented by older Windows OpenGL drivers.  In particular, some
// NVIDIA drivers evaluate an iteration after the uniform-controlled `break`,
// normalize an unused zero direction, and contaminate the result with NaNs.
// Keep every array subscript constant and avoid normalize(vec2(0.0)).
vec2 safe_direction(vec2 dir) {
    float len2 = dot(dir, dir);
    return len2 > 1e-8 ? dir * inversesqrt(len2) : vec2(1.0, 0.0);
}

// Total airborne sand at wp: sum of engine wakes from every vehicle. Each wake
// streams outward along the vehicle's exhaust. Rocks kick up no dust.
float dust_disturbance(vec2 wp) {
    float amt = 0.0;

    if (u_ship_count > 0) {
        float R = u_ships[0].z * SHIP_R_SCALE;
        float thr = u_ship_thrust[0]; // signed: + forward, − braking
        float mag = clamp(abs(thr), 0.0, 1.0);
        // Coasting → short round puff. Accelerating → long trail behind; braking
        // → shorter trail flipped to the front (retro thrust).
        float plume = mix(DUST_SHIP_PLUME_IDLE,
                thr >= 0.0 ? DUST_SHIP_PLUME_FULL : DUST_SHIP_PLUME_BRAKE,
                mag);
        vec2 dir = safe_direction(u_ship_dir[0]) * (thr >= 0.0 ? 1.0 : -1.0);
        amt += engine_wake(wp, u_ships[0].xy, dir, R, plume, 2.6);
    }

    if (u_ship_count > 1) {
        float R = u_ships[1].z * SHIP_R_SCALE;
        float thr = u_ship_thrust[1];
        float mag = clamp(abs(thr), 0.0, 1.0);
        float plume = mix(DUST_SHIP_PLUME_IDLE,
                thr >= 0.0 ? DUST_SHIP_PLUME_FULL : DUST_SHIP_PLUME_BRAKE,
                mag);
        vec2 dir = safe_direction(u_ship_dir[1]) * (thr >= 0.0 ? 1.0 : -1.0);
        amt += engine_wake(wp, u_ships[1].xy, dir, R, plume, 2.6);
    }

    float pulse = 0.6 + 0.4 * sin(u_time * 6.0); // exhaust puffs
    if (u_rocket_count > 0)
        amt += engine_wake(wp, u_rockets[0].xy, safe_direction(u_rocket_dir[0]), u_rockets[0].z * ROCKET_R_SCALE, DUST_ROCKET_PLUME, 2.2) * pulse;
    if (u_rocket_count > 1)
        amt += engine_wake(wp, u_rockets[1].xy, safe_direction(u_rocket_dir[1]), u_rockets[1].z * ROCKET_R_SCALE, DUST_ROCKET_PLUME, 2.2) * pulse;
    if (u_rocket_count > 2)
        amt += engine_wake(wp, u_rockets[2].xy, safe_direction(u_rocket_dir[2]), u_rockets[2].z * ROCKET_R_SCALE, DUST_ROCKET_PLUME, 2.2) * pulse;
    if (u_rocket_count > 3)
        amt += engine_wake(wp, u_rockets[3].xy, safe_direction(u_rocket_dir[3]), u_rockets[3].z * ROCKET_R_SCALE, DUST_ROCKET_PLUME, 2.2) * pulse;

    return amt;
}

// ── Entry point ───────────────────────────────────────────────────────────────

void main() {
    vec2 wp = frag_to_world(gl_FragCoord.xy);
    vec3 col = shade_desert(wp);

    // Additive blowing-sand grains streaming off each vehicle's engines.
    float dust = clamp(dust_disturbance(wp), 0.0, 1.0);
    col += u_palette_b * dust * DUST_BRIGHTNESS;

    // Reinhard tone map
    col = col / (col + 1.0);

    gl_FragColor = vec4(col, 1.0);
}
