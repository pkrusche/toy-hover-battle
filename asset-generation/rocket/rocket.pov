#version 3.7;
#include "colors.inc"
#include "textures.inc"

global_settings { assumed_gamma 1.0 }

#ifndef(YawAngle)
  #declare YawAngle = 0;
#end

#ifndef(AccentRed)
  #declare AccentRed = 0.92;
#end

#ifndef(AccentGreen)
  #declare AccentGreen = 0.75;
#end

#ifndef(AccentBlue)
  #declare AccentBlue = 0.05;
#end

#declare AccentColor = color srgb <AccentRed, AccentGreen, AccentBlue>;

// -----------------------------------------------------------------------------
// Framing – same isometric camera as ship
// -----------------------------------------------------------------------------
background { color srgbt <0,0,0,1> }

camera {
  orthographic
  location <34.641, 34.641, -34.641>
  look_at  <0, 0, 0>
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
// Materials – same visual language as ship
// -----------------------------------------------------------------------------
#declare RustFlecks = texture {
  pigment {
    granite
    color_map {
      [0.00 color rgbt <0.16, 0.07, 0.025, 0.08>]
      [0.16 color rgbt <0.38, 0.18, 0.07,  0.18>]
      [0.28 color rgbt <0.12, 0.10, 0.08,  0.50>]
      [0.36 color rgbt <1.00, 1.00, 1.00,  1.00>]
      [1.00 color rgbt <1.00, 1.00, 1.00,  1.00>]
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
      [0.20 color rgbt <0.04,  0.04,  0.04,  0.28>]
      [0.42 color rgbt <0.18,  0.17,  0.15,  0.55>]
      [0.56 color rgbt <1.00,  1.00,  1.00,  1.00>]
      [1.00 color rgbt <1.00,  1.00,  1.00,  1.00>]
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
    normal { dents 0.55 scale 0.32 }
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
    normal { bumps 0.35 scale 0.22 }
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
    normal { dents 0.26 scale 0.24 }
    finish  { diffuse 0.90 ambient 0.025 specular 0.055 roughness 0.24 }
  }
  texture { DustFilm }
  texture { RustFlecks }
  texture { SootStreaks }

#declare EngineGlowTex = texture {
  pigment { color srgb <0.28, 0.92, 1.00> }
  finish  { diffuse 0.25 ambient 0.60 specular 0.15 roughness 0.03 }
}

// -----------------------------------------------------------------------------
// Rocket
// Model forward direction: -Z (nose tip at -Z, engine nozzle at +Z)
// Total length ~3.65 units to fill the 32x32 frame alongside ships.
// -----------------------------------------------------------------------------
#declare Rocket =
union {

  // -------------------------------------------------------------------------
  // NOSE CONE – sharp spike
  // -------------------------------------------------------------------------
  cone {
    <0, 0, -2.0>, 0.01,
    <0, 0, -1.0>, 0.26
    texture { AccentPaint }
  }

  // Nose-body collar (accent band at shoulder)
  cylinder { <0, 0, -1.02>, <0, 0, -0.78>, 0.27
    texture { AccentPaint }
  }

  // -------------------------------------------------------------------------
  // MAIN BODY – cylindrical hull
  // -------------------------------------------------------------------------
  cylinder { <0, 0, -0.78>, <0, 0, 0.88>, 0.24
    texture { HullMetal }
  }

  // Mid accent ring
  cylinder { <0, 0, -0.06>, <0, 0, 0.14>, 0.255
    texture { AccentPaint }
  }

  // -------------------------------------------------------------------------
  // ENGINE SECTION
  // -------------------------------------------------------------------------
  // Shroud taper
  cone { <0, 0, 0.88>, 0.24, <0, 0, 1.20>, 0.18
    texture { DarkMetal }
  }

  // Nozzle bell
  cone { <0, 0, 1.20>, 0.18, <0, 0, 1.55>, 0.28
    texture { DarkMetal }
  }

  // -------------------------------------------------------------------------
  // STABILISER FINS – four rectangular fins at 90° intervals
  // -------------------------------------------------------------------------
  // +X fin
  box { <0.26, -0.04, 0.28>, < 0.52, 0.04, 1.18>
    texture { AccentPaint }
  }
  // -X fin
  box { <-0.52, -0.04, 0.28>, <-0.26, 0.04, 1.18>
    texture { AccentPaint }
  }
  // +Y fin
  box { <-0.04,  0.26, 0.28>, < 0.04, 0.52, 1.18>
    texture { AccentPaint }
  }
  // -Y fin
  box { <-0.04, -0.52, 0.28>, < 0.04, -0.26, 1.18>
    texture { AccentPaint }
  }

  // -------------------------------------------------------------------------
  // ENGINE GLOW
  // -------------------------------------------------------------------------
  sphere { <0, 0, 1.65>, 0.18
    texture { EngineGlowTex }
    no_shadow
  }

}

object {
  Rocket
  rotate <0, YawAngle, 0>
}
