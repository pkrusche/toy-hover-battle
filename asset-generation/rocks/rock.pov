#version 3.7;

global_settings {
  assumed_gamma 1.0
  noise_generator 2
  max_trace_level 4
}

// -----------------------------------------------------------------------------
// Render parameters
// -----------------------------------------------------------------------------
#ifndef(SeedValue)
  #declare SeedValue = 1201;
#end

#ifndef(Rock_Radius)
  #declare Rock_Radius = 1.00;
#end

#ifndef(Macro_Bumps)
  #declare Macro_Bumps = 7;
#end

#ifndef(Roughness_Amount)
  #declare Roughness_Amount = 0.18;
#end

#ifndef(Asymmetry)
  #declare Asymmetry = 0.18;
#end

#ifndef(Vertical_Scale)
  #declare Vertical_Scale = 0.65;
#end

#ifndef(Floor_Cut)
  #declare Floor_Cut = -0.38;
#end

#ifndef(Detail_Scale)
  #declare Detail_Scale = 0.12;
#end

#ifndef(Detail_Amount)
  #declare Detail_Amount = 0.15;
#end

#ifndef(Color_Variation)
  #declare Color_Variation = 1;
#end

#ifndef(Use_Outline)
  #declare Use_Outline = 0;
#end

#ifndef(Outline_Width)
  #declare Outline_Width = 0.04;
#end

#ifndef(Outline_Shift)
  #declare Outline_Shift = 0.08;
#end

#ifndef(Mask_Mode)
  #declare Mask_Mode = 0;
#end

#ifndef(ViewHeight)
  #declare ViewHeight = 1.75;
#end

// -----------------------------------------------------------------------------
// Framing
// -----------------------------------------------------------------------------
#declare CamDist = 10;
#declare CamPos = vnormalize(<-1, 1, -1>) * CamDist;
#declare ViewDir = vnormalize(<0, 0, 0> - CamPos);

background { color srgbt <0, 0, 0, 1> }

camera {
  orthographic
  location CamPos
  look_at 0
  sky y
  up y * ViewHeight
  right x * ViewHeight * image_width / image_height
}

// -----------------------------------------------------------------------------
// Lighting
// -----------------------------------------------------------------------------
light_source {
  <300, 500, -300>
  color srgb <1.00, 0.98, 0.95>
  parallel
  point_at 0
}

light_source {
  <-150, 120, -80>
  color srgb <0.28, 0.30, 0.34>
  shadowless
}

// -----------------------------------------------------------------------------
// Utility macros
// -----------------------------------------------------------------------------
#macro SignedRand(Stream)
  (rand(Stream) * 2 - 1)
#end

// -----------------------------------------------------------------------------
// Materials — sandstone palettes
// -----------------------------------------------------------------------------
#declare Texture_Stream = seed(SeedValue + 7919);
#declare Palette_Pick = rand(Texture_Stream);
#declare Grain_Scale = 0.62 + rand(Texture_Stream) * 0.46;
#declare Grain_Rotate = <rand(Texture_Stream) * 360, rand(Texture_Stream) * 360, rand(Texture_Stream) * 360>;
#declare Grain_Translate = <rand(Texture_Stream) * 12, rand(Texture_Stream) * 12, rand(Texture_Stream) * 12>;
#declare Relief_Amount = Detail_Amount * (0.75 + rand(Texture_Stream) * 0.65);
#declare Relief_Scale = Detail_Scale * (0.75 + rand(Texture_Stream) * 0.70);

#if (Color_Variation)
  #if (Palette_Pick < 0.30)
    // Buff / cream sandstone
    #declare Rock_Color_0 = <0.52, 0.43, 0.28>;
    #declare Rock_Color_1 = <0.66, 0.56, 0.37>;
    #declare Rock_Color_2 = <0.78, 0.68, 0.48>;
    #declare Rock_Color_3 = <0.88, 0.80, 0.60>;
  #elseif (Palette_Pick < 0.58)
    // Red / rust sandstone
    #declare Rock_Color_0 = <0.42, 0.18, 0.10>;
    #declare Rock_Color_1 = <0.58, 0.28, 0.16>;
    #declare Rock_Color_2 = <0.70, 0.40, 0.22>;
    #declare Rock_Color_3 = <0.78, 0.52, 0.30>;
  #elseif (Palette_Pick < 0.80)
    // Orange / tan sandstone
    #declare Rock_Color_0 = <0.50, 0.35, 0.18>;
    #declare Rock_Color_1 = <0.64, 0.48, 0.26>;
    #declare Rock_Color_2 = <0.76, 0.60, 0.36>;
    #declare Rock_Color_3 = <0.86, 0.72, 0.46>;
  #else
    // Warm brown sandstone
    #declare Rock_Color_0 = <0.30, 0.20, 0.12>;
    #declare Rock_Color_1 = <0.46, 0.32, 0.18>;
    #declare Rock_Color_2 = <0.60, 0.44, 0.26>;
    #declare Rock_Color_3 = <0.72, 0.56, 0.36>;
  #end
#else
  #declare Rock_Color_0 = <0.48, 0.38, 0.24>;
  #declare Rock_Color_1 = <0.62, 0.50, 0.32>;
  #declare Rock_Color_2 = <0.74, 0.62, 0.42>;
  #declare Rock_Color_3 = <0.84, 0.74, 0.54>;
#end

#declare Rock_Texture =
// Base grain
texture {
  pigment {
    granite
    color_map {
      [0.00 color srgb Rock_Color_0]
      [0.35 color srgb Rock_Color_1]
      [0.70 color srgb Rock_Color_2]
      [1.00 color srgb Rock_Color_3]
    }
    turbulence 0.35
    lambda 2
    omega 0.55
    octaves 5
    scale Grain_Scale
    rotate Grain_Rotate
    translate Grain_Translate
  }
  normal {
    granite Relief_Amount
    turbulence 0.25
    lambda 2
    omega 0.55
    octaves 5
    scale Relief_Scale
    rotate Grain_Rotate
    translate Grain_Translate
  }
  finish {
    ambient 0
    diffuse 0.90
    brilliance 1.0
    specular 0.04
    roughness 0.25
  }
}
// Subtle sedimentary banding
texture {
  pigment {
    wood
    color_map {
      [0.00 color srgbt <Rock_Color_0.x, Rock_Color_0.y, Rock_Color_0.z, 0.72>]
      [0.50 color srgbt <1.00, 1.00, 1.00, 1.00>]
      [1.00 color srgbt <Rock_Color_1.x, Rock_Color_1.y, Rock_Color_1.z, 0.80>]
    }
    turbulence 0.08
    scale <4.0, 0.40, 4.0>
    rotate Grain_Rotate
    translate Grain_Translate
  }
}
// Fine surface variation
texture {
  pigment {
    bozo
    color_map {
      [0.00 color srgbt <0.06, 0.05, 0.04, 0.55>]
      [0.40 color srgbt <0.10, 0.08, 0.06, 0.75>]
      [0.70 color srgbt <1.00, 1.00, 1.00, 1.00>]
      [1.00 color srgbt <Rock_Color_3.x, Rock_Color_3.y, Rock_Color_3.z, 0.88>]
    }
    turbulence 0.60
    scale (Grain_Scale * 0.55)
    rotate Grain_Rotate
    translate (Grain_Translate + <3.1, 5.7, 2.4>)
  }
}

#declare Outline_Texture = texture {
  pigment { color srgb <0.08, 0.07, 0.06> }
  finish { ambient 0 diffuse 1 specular 0 }
}

// -----------------------------------------------------------------------------
// Rock generator — rounded sandstone form, no spikes
// -----------------------------------------------------------------------------
#macro MakeRock(R, NumBumps, RoughAmt, Asym, YScale, CutY, SeedInt)
  #local S = seed(SeedInt);

  #local RoundRock =
  blob {
    threshold 0.72

    // Dominant central sphere keeps the form smooth and convex
    sphere { 0, R * (0.82 + RoughAmt * 0.06), 1.12 }

    #local I = 0;
    #while (I < NumBumps)
      #local A = vnormalize(
        <SignedRand(S), SignedRand(S) * 0.65, SignedRand(S)>
        + <0.001, 0.001, 0.001>
      );

      // Bumps stay close to centre so they swell rather than spike
      #local Dist = R * (0.16 + rand(S) * (0.32 + Asym * 0.15));
      #local C = <
        A.x * Dist * (1 + Asym),
        A.y * Dist * (1 - Asym * 0.40),
        A.z * Dist * (1 + Asym * 0.30)
      >;

      #local Rad = R * (0.28 + rand(S) * (0.26 + RoughAmt * 0.16));
      #local Str = 0.55 + rand(S) * 0.55;

      sphere { C, Rad, Str }

      #local I = I + 1;
    #end

    sphere { <0.18 * R, -0.05 * R, 0.18 * R>, 0.22 * R, -0.20 }
  };

  intersection {
    object { RoundRock scale <1, YScale, 1> }
    sphere { 0, R }
    box { <-R * 1.10, R * CutY, -R * 1.10>, <R * 1.10, R * 1.10, R * 1.10> }
    texture { Rock_Texture }
  }
#end

// -----------------------------------------------------------------------------
// Render selection
// -----------------------------------------------------------------------------
#if (Mask_Mode)
  sphere {
    0, Rock_Radius
    pigment { color srgb <1, 1, 1> }
    finish { ambient 1 diffuse 0 }
  }
#else
  #if (Use_Outline)
    sphere {
      ViewDir * Outline_Shift, Rock_Radius * (1 + Outline_Width)
      texture { Outline_Texture }
    }
  #end

  MakeRock(
    Rock_Radius,
    Macro_Bumps,
    Roughness_Amount,
    Asymmetry,
    Vertical_Scale,
    Floor_Cut,
    SeedValue
  )
#end
