#version 3.7;
#include "colors.inc"
#include "textures.inc"

global_settings { assumed_gamma 1.0 }

#ifndef(YawAngle)
  #declare YawAngle = 0;
#end

#ifndef(AccentRed)
  #declare AccentRed = 0.86;
#end

#ifndef(AccentGreen)
  #declare AccentGreen = 0.24;
#end

#ifndef(AccentBlue)
  #declare AccentBlue = 0.20;
#end

#declare AccentColor = color srgb <AccentRed, AccentGreen, AccentBlue>;

// -----------------------------------------------------------------------------
// Framing
// -----------------------------------------------------------------------------
#declare CamDir    = <1,1,-1>/sqrt(3);    // true isometric view direction

background { color srgbt <0,0,0,1> }      // fully transparent when rendered with +UA

camera {
  orthographic
  location <34.641, 34.641, -34.641>
  look_at  <0, 0.15, 0>
  sky y
  up    y * 4.0
  right x * 4.0 * (image_width / image_height)
}

// -----------------------------------------------------------------------------
// Lighting
// -----------------------------------------------------------------------------
light_source {
  <60, 90, -40>
  color srgb <1.00, 0.98, 0.95>
}

light_source {
  <-45, 35, -15>
  color srgb <0.38, 0.44, 0.55>
  shadowless
}

light_source {
  <0, 55, 55>
  color srgb <0.18, 0.20, 0.26>
  shadowless
}

// -----------------------------------------------------------------------------
// Materials
// -----------------------------------------------------------------------------
#declare RustFlecks = texture {
  pigment {
    granite
    color_map {
      [0.00 color rgbt <0.16, 0.07, 0.025, 0.08>]
      [0.16 color rgbt <0.38, 0.18, 0.07, 0.18>]
      [0.28 color rgbt <0.12, 0.10, 0.08, 0.50>]
      [0.36 color rgbt <1.00, 1.00, 1.00, 1.00>]
      [1.00 color rgbt <1.00, 1.00, 1.00, 1.00>]
    }
    frequency 22
    turbulence 0.55
    scale 0.55
  }
}

#declare SootStreaks = texture {
  pigment {
    bozo
    color_map {
      [0.00 color rgbt <0.015, 0.014, 0.012, 0.10>]
      [0.20 color rgbt <0.04, 0.04, 0.04, 0.28>]
      [0.42 color rgbt <0.18, 0.17, 0.15, 0.55>]
      [0.56 color rgbt <1.00, 1.00, 1.00, 1.00>]
      [1.00 color rgbt <1.00, 1.00, 1.00, 1.00>]
    }
    turbulence 0.9
    scale <0.55, 0.22, 1.6>
  }
}

#declare DustFilm = texture {
  pigment {
    wrinkles
    color_map {
      [0.00 color rgbt <0.30, 0.27, 0.22, 0.72>]
      [0.45 color rgbt <0.17, 0.15, 0.13, 0.82>]
      [0.72 color rgbt <1.00, 1.00, 1.00, 1.00>]
      [1.00 color rgbt <1.00, 1.00, 1.00, 1.00>]
    }
    turbulence 0.35
    scale 0.42
  }
}

#declare HullMetal =
  texture {
    pigment {
      granite
      color_map {
        [0.00 color srgb <0.39, 0.41, 0.42>]
        [0.45 color srgb <0.50, 0.51, 0.50>]
        [1.00 color srgb <0.28, 0.29, 0.30>]
      }
      turbulence 0.25
      scale 0.8
    }
    normal {
      dents 0.55
      scale 0.32
    }
    finish  { diffuse 0.88 ambient 0.025 specular 0.08 roughness 0.18 }
  }
  texture { DustFilm }
  texture { RustFlecks }
  texture { SootStreaks }

#declare DarkMetal =
  texture {
    pigment {
      granite
      color_map {
        [0.00 color srgb <0.10, 0.11, 0.12>]
        [0.55 color srgb <0.18, 0.19, 0.21>]
        [1.00 color srgb <0.07, 0.08, 0.09>]
      }
      turbulence 0.35
      scale 0.5
    }
    normal {
      bumps 0.35
      scale 0.22
    }
    finish  { diffuse 0.86 ambient 0.02 specular 0.06 roughness 0.22 }
  }
  texture { DustFilm }
  texture { SootStreaks }

#declare AccentPaint =
  texture {
    pigment {
      gradient y
      color_map {
        [0.00 color AccentColor * 0.58]
        [0.48 color AccentColor * 0.92]
        [1.00 color AccentColor * 0.66]
      }
      turbulence 0.18
      scale 0.9
    }
    normal {
      dents 0.26
      scale 0.24
    }
    finish  { diffuse 0.90 ambient 0.025 specular 0.055 roughness 0.24 }
  }
  texture { DustFilm }
  texture { RustFlecks }
  texture { SootStreaks }

#declare CanopyTex = texture {
    pigment {
        color rgb <0.075, 0.11, 0.17>
    }
    normal {
        waves 0.22
        frequency 2
        scale 0.12
    }
    finish {
        reflection { 0.12 }
        specular 0.22
        roughness 0.08
        ambient 0.055
        diffuse 0.66
    }
}
texture { DustFilm }
texture { SootStreaks }

#declare EngineGlowTex = texture {
  pigment { color srgb <0.28, 0.92, 1.00> }
  finish  { diffuse 0.25 ambient 0.60 specular 0.15 roughness 0.03 }
}

// -----------------------------------------------------------------------------
// Reusable parts
// -----------------------------------------------------------------------------
#declare LeftWing =
union {
  box  { <-1.05,-0.07,-0.95>, < 1.05,0.07, 0.90> }
  cone { < 1.05,0,-0.75>, 0.18, < 1.55,0,-1.15>, 0.00 }
  cone { < 1.05,0, 0.70>, 0.15, < 1.35,0, 1.05>, 0.00 }
  texture { AccentPaint }
}

#declare EnginePod =
union {
  cylinder { <0,0,0>, <0,0,-0.75>, 0.23 }
  cone     { <0,0,-0.75>, 0.25, <0,0,-1.08>, 0.15 }
  texture { DarkMetal }
}

#declare EngineGlow =
union {
  cylinder { <0,0,0>, <0,0,-0.18>, 0.14 texture { EngineGlowTex } no_shadow }
}

// -----------------------------------------------------------------------------
// Main ship
// Model forward direction: +Z
// -----------------------------------------------------------------------------
#declare Ship =
union {
  // -------------------------------------------------------------------------
  // FRONT TIP (nose spike)
  // -------------------------------------------------------------------------
  cone {
    <0, 0.0, -1.0>, 0.6,   // base (attached to body)
    <0, 0.0, -1.8>, 0.02    // sharp tip forward
    scale <1, 0.5, 1>
    texture { AccentPaint }
  }

  // -------------------------------------------------------------------------
  // MAIN BODY (flattened disc / saucer)
  // -------------------------------------------------------------------------
  object {
    sphere { <0,0,0>, 1 }
    scale <1.4, 0.55, 1.2>
    texture { HullMetal }
  }

  // central circular panel (top/front visible)
  torus {
    0.35, 0.08
    rotate <90,0,0>
    translate <0,0.05,-0.1>
    texture { DarkMetal }
  }

  // -------------------------------------------------------------------------
  // COCKPIT (top bump)
  // -------------------------------------------------------------------------
  sphere {
    <0, 0.85, -0.2>, 0.35
    scale <1.2, 0.7, 1.0>
    texture { CanopyTex }
  }

// -------------------------------------------------------------------------
// TOP FIN (trapezoid, matches sketch)
// -------------------------------------------------------------------------
object {
  prism {
    linear_sweep
    linear_spline
    0, 0.1,   // thickness (Z extrusion)
    4,
    <-0.5, 0.0>,   // base left
    < 0.5, 0.0>,   // base right
    < 0.2, 0.6>,   // top right (narrower)
    <-0.1, 0.7>    // top left
  }
  rotate <270,90,0>   // stand upright
  translate <0,0.4,0.6>
  texture { AccentPaint }
}


  // -------------------------------------------------------------------------
  // SIDE WINGS (flat trapezoid style)
  // -------------------------------------------------------------------------
  union {
    box { <-1.6,-0.05,-0.4>, <-0.7,0.05,0.4> }
    box { < 0.7,-0.05,-0.4>, < 1.6,0.05,0.4> }
    texture { AccentPaint }
  }

  // -------------------------------------------------------------------------
  // SIDE POD BULGES (seen in right view)
  // -------------------------------------------------------------------------
  sphere { <-1.1, 0.0, 0.0>, 0.25 texture { DarkMetal } }
  sphere { < 1.1, 0.0, 0.0>, 0.25 texture { DarkMetal } }

  // -------------------------------------------------------------------------
  // REAR ENGINE BLOCK (matches rear sketch)
  // -------------------------------------------------------------------------
  union {
    box { <-0.5,-0.2,1.0>, <0.5,0.3,1.4> texture { DarkMetal } }

    // dual engines
    cylinder { <-0.25,0.05,1.4>, <-0.25,0.05,1.8>, 0.18 }
    cylinder { < 0.25,0.05,1.4>, < 0.25,0.05,1.8>, 0.18 }

    // glow
    sphere { <-0.25,0.05,1.9>, 0.14 texture { EngineGlowTex } no_shadow }
    sphere { < 0.25,0.05,1.9>, 0.14 texture { EngineGlowTex } no_shadow }
  }

}

object {
  Ship
  rotate <0, YawAngle, 0>
}
