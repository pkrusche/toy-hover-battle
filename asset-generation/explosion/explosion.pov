#version 3.7;

global_settings {
  assumed_gamma 1.0
  ambient_light rgb 0
  noise_generator 2
  max_trace_level 12
  max_intersections 128
}

// -----------------------------------------------------------------------------
// Render parameters
// -----------------------------------------------------------------------------
#ifndef(Preview_Ground)
  #declare Preview_Ground = 0;
#end

#ifndef(Smoke_Enable)
  #declare Smoke_Enable = 1;
#end

#ifndef(Smoke_NoShadow)
  #declare Smoke_NoShadow = 1;
#end

#ifndef(Use_Media_Core)
  #declare Use_Media_Core = 0;
#end

#ifndef(Explosion_Seed)
  #declare Explosion_Seed = 1701;
#end

#ifndef(Explosion_Start)
  #declare Explosion_Start = 0.00;
#end

#ifndef(Explosion_Duration)
  #declare Explosion_Duration = 0.72;
#end

#ifndef(Explosion_MinRadius)
  #declare Explosion_MinRadius = 0.00;
#end

#ifndef(Explosion_MaxRadius)
  #declare Explosion_MaxRadius = 6.50;
#end

#ifndef(Explosion_ExpansionRate)
  #declare Explosion_ExpansionRate = 4.00;
#end

#ifndef(Explosion_DissipationGamma)
  #declare Explosion_DissipationGamma = 1.65;
#end

#ifndef(Fire_Count)
  #declare Fire_Count = 280;
#end

#ifndef(Fire_BirthJitter)
  #declare Fire_BirthJitter = 0.18;
#end

#ifndef(Fire_LifetimeMin)
  #declare Fire_LifetimeMin = 0.18;
#end

#ifndef(Fire_LifetimeMax)
  #declare Fire_LifetimeMax = 0.42;
#end

#ifndef(Fire_SizeMin)
  #declare Fire_SizeMin = 0.08;
#end

#ifndef(Fire_SizeMax)
  #declare Fire_SizeMax = 0.26;
#end

#ifndef(Fire_VelocityMin)
  #declare Fire_VelocityMin = 0.65;
#end

#ifndef(Fire_VelocityMax)
  #declare Fire_VelocityMax = 1.25;
#end

#ifndef(Fire_UpBias)
  #declare Fire_UpBias = 0.20;
#end

#ifndef(Fire_Turbulence)
  #declare Fire_Turbulence = 0.35;
#end

#ifndef(Fire_NoiseFrequency)
  #declare Fire_NoiseFrequency = 2.75;
#end

#ifndef(Smoke_Seed)
  #declare Smoke_Seed = 24011;
#end

#ifndef(Smoke_Count)
  #declare Smoke_Count = 70;
#end

#ifndef(Smoke_Delay)
  #declare Smoke_Delay = 0.10;
#end

#ifndef(Smoke_LifetimeMin)
  #declare Smoke_LifetimeMin = 0.45;
#end

#ifndef(Smoke_LifetimeMax)
  #declare Smoke_LifetimeMax = 0.95;
#end

#ifndef(Smoke_SizeMin)
  #declare Smoke_SizeMin = 0.22;
#end

#ifndef(Smoke_SizeMax)
  #declare Smoke_SizeMax = 1.10;
#end

#ifndef(Smoke_VelocityMin)
  #declare Smoke_VelocityMin = 0.18;
#end

#ifndef(Smoke_VelocityMax)
  #declare Smoke_VelocityMax = 0.52;
#end

#ifndef(Smoke_Rise)
  #declare Smoke_Rise = 1.80;
#end

#ifndef(Smoke_Turbulence)
  #declare Smoke_Turbulence = 0.70;
#end

#ifndef(Smoke_Octaves)
  #declare Smoke_Octaves = 6;
#end

#ifndef(Smoke_Omega)
  #declare Smoke_Omega = 0.55;
#end

#ifndef(Smoke_Lambda)
  #declare Smoke_Lambda = 2.00;
#end

#ifndef(Smoke_Opacity)
  #declare Smoke_Opacity = 0.70;
#end

#ifndef(ViewHeight)
  #declare ViewHeight = 14.00;
#end

#declare Explosion_Center = <0, 0, 0>;

// -----------------------------------------------------------------------------
// Framing
// -----------------------------------------------------------------------------
#declare Iso_Offset = vnormalize(<-1, 1, -1>) * 24;

background { color srgbt <0, 0, 0, 1> }

camera {
  orthographic
  location Explosion_Center + Iso_Offset
  look_at Explosion_Center
  sky y
  up y * ViewHeight
  right x * ViewHeight * image_width / image_height
}

// -----------------------------------------------------------------------------
// Lighting
// -----------------------------------------------------------------------------
light_source {
  <-18, 22, -18>
  color srgb <1.10, 1.00, 0.92>
  fade_distance 28
  fade_power 2
}

light_source {
  <20, 12, 18>
  color srgb <0.35, 0.45, 0.70>
  fade_distance 30
  fade_power 2
}

light_source {
  <0, 24, -26>
  color srgb <0.18, 0.12, 0.10>
  shadowless
}

#if (Preview_Ground)
  plane {
    y, -0.45
    texture {
      pigment { color srgb <0.07, 0.07, 0.08> }
      finish { diffuse 0.85 specular 0.05 roughness 0.08 }
    }
  }
#end

// -----------------------------------------------------------------------------
// Utility macros
// -----------------------------------------------------------------------------
#macro Clamp(X, A, B)
  min(max((X), (A)), (B))
#end

#macro Saturate(X)
  Clamp((X), 0.0, 1.0)
#end

#macro Lerp(A, B, T)
  ((A) + ((B) - (A)) * (T))
#end

#macro EaseOutExp(T, K)
  (1.0 - exp(-(K) * (T)))
#end

#macro SafeUnit(V)
  vnormalize((V) + <1e-6, 2e-6, 3e-6>)
#end

#macro RandomDir(S, UpBias)
  #local V = <2 * rand(S) - 1, 2 * rand(S) - 1 + UpBias, 2 * rand(S) - 1>;
  SafeUnit(V)
#end

#macro FireParticle(P, R, Heat)
  sphere {
    P, R
    texture {
      pigment {
        spherical
        color_map {
          [0.00 color srgb <1.35, 1.20, 1.00>]
          [0.25 color srgb <1.40, 0.75, 0.15>]
          [0.65 color srgb <1.00, 0.20, 0.03>]
          [1.00 color srgbt <0.20, 0.02, 0.00, 0.75>]
        }
        scale R * 1.25
      }
      finish {
        emission <2.8, 1.4, 0.45> * Heat
        diffuse 0.12
        specular 0.10
        roughness 0.05
      }
    }
    no_shadow
  }
#end

#macro SmokePuff(P, R, Opacity, SeedValue)
  #local SR = seed(SeedValue);
  #local TMid = Clamp(1.0 - 0.38 * Opacity, 0.0, 1.0);
  #local TEdge = Clamp(1.0 - 0.78 * Opacity, 0.0, 1.0);

  blob {
    threshold 0.55

    #local J = 0;
    #while (J < 4)
      sphere {
        P + <rand(SR) - 0.5, rand(SR) - 0.5, rand(SR) - 0.5> * R * 0.9,
        R * (0.45 + 0.40 * rand(SR)),
        1.20
      }
      #local J = J + 1;
    #end

    texture {
      pigment {
        bozo
        color_map {
          [0.00 color srgbt <0.10, 0.10, 0.10, 1.00>]
          [0.45 color srgbt <0.14, 0.14, 0.14, TMid>]
          [1.00 color srgbt <0.05, 0.05, 0.05, TEdge>]
        }
        turbulence <Smoke_Turbulence, Smoke_Turbulence, Smoke_Turbulence>
        octaves Smoke_Octaves
        omega Smoke_Omega
        lambda Smoke_Lambda
        scale R * 1.50
      }
      finish {
        diffuse 0.18
        specular 0
      }
    }

    #if (Smoke_NoShadow)
      no_shadow
    #end
  }
#end

// -----------------------------------------------------------------------------
// Master time curve
// -----------------------------------------------------------------------------
#declare T_Global = Saturate((clock - Explosion_Start) / Explosion_Duration);
#declare Blast_R = Lerp(
  Explosion_MinRadius,
  Explosion_MaxRadius,
  EaseOutExp(T_Global, Explosion_ExpansionRate)
);
#declare Blast_A = pow(1.0 - T_Global, Explosion_DissipationGamma);

// -----------------------------------------------------------------------------
// Optional volumetric core
// -----------------------------------------------------------------------------
#if ((Use_Media_Core != 0) & (T_Global > 0.0))
  sphere {
    Explosion_Center, Lerp(0.25, 1.20, T_Global)
    hollow
    texture {
      pigment { color srgbt <1, 1, 1, 1> }
      finish { emission 0 diffuse 0 }
    }
    interior {
      media {
        method 3
        intervals 1
        samples 8, 20
        aa_level 4
        aa_threshold 0.08
        emission <2.6, 1.4, 0.45> * Blast_A
        density {
          spherical
          color_map {
            [0.00 color rgb 0]
            [0.30 color rgb <0.85, 0.15, 0.03>]
            [0.70 color rgb <1.00, 0.60, 0.18>]
            [1.00 color rgb <1.10, 1.05, 0.95>]
          }
          turbulence <0.75, 0.75, 0.75>
          octaves Smoke_Octaves
          omega Smoke_Omega
          lambda Smoke_Lambda
          scale 1.10
        }
      }
    }
    no_shadow
  }
#end

// -----------------------------------------------------------------------------
// Fire particles
// -----------------------------------------------------------------------------
#local I = 0;
#while (I < Fire_Count)
  #local PS = seed(Explosion_Seed + I * 7919);
  #local Dir = RandomDir(PS, Fire_UpBias);
  #local Birth = rand(PS) * Fire_BirthJitter;
  #local Life = Lerp(Fire_LifetimeMin, Fire_LifetimeMax, rand(PS));
  #local Speed = Lerp(Fire_VelocityMin, Fire_VelocityMax, rand(PS));
  #local SizeBias = rand(PS);
  #local HeatBias = rand(PS);
  #local ShellBias = rand(PS);
  #local Phase = rand(PS) * 7.0;
  #local Age = (T_Global - Birth) / Life;

  #if ((Age > 0.0) & (Age < 1.0))
    #local Age01 = Saturate(Age);
    #local ShellR = Blast_R * (0.65 + 0.55 * ShellBias) * Speed;
    #local Drift = Fire_Turbulence *
      vturbulence(
        Smoke_Lambda,
        Smoke_Omega,
        Smoke_Octaves,
        Dir * Fire_NoiseFrequency + <Phase, T_Global * 4.0, 0>
      );
    #local Pos = Explosion_Center + Dir * ShellR + Drift * Age01 * 0.60;
    #local Heat = pow(1.0 - Age01, Explosion_DissipationGamma) *
      Lerp(0.85, 1.35, HeatBias);
    #local Rad = Lerp(Fire_SizeMin, Fire_SizeMax, SizeBias) * (0.45 + 0.90 * Heat);

    FireParticle(Pos, Rad, Heat)
  #end

  #local I = I + 1;
#end

// -----------------------------------------------------------------------------
// Smoke puffs
// -----------------------------------------------------------------------------
#if (Smoke_Enable)
  #local I = 0;
  #while (I < Smoke_Count)
    #local SS = seed(Smoke_Seed + I * 3571);
    #local Dir = RandomDir(SS, 0.45);
    #local Birth = Smoke_Delay + rand(SS) * 0.35;
    #local Life = Lerp(Smoke_LifetimeMin, Smoke_LifetimeMax, rand(SS));
    #local Vel = Lerp(Smoke_VelocityMin, Smoke_VelocityMax, rand(SS));
    #local SizeBias = rand(SS);
    #local Attach = rand(SS);
    #local PuffSeed = Smoke_Seed + I * 157 + 11;
    #local Age = (T_Global - Birth) / Life;

    #if ((Age > 0.0) & (Age < 1.0))
      #local Age01 = Saturate(Age);
      #local BaseR = Lerp(Smoke_SizeMin, Smoke_SizeMax, Age01) *
        Lerp(0.75, 1.25, SizeBias);

      #local Advect =
        Dir * (Blast_R * (0.55 + 0.55 * Attach) * 0.65)
        + y * (Smoke_Rise * Vel * Age01);

      #local Curl = 0.35 *
        vturbulence(
          Smoke_Lambda,
          Smoke_Omega,
          Smoke_Octaves,
          Dir * 2.0 + <0, T_Global * 3.0, PuffSeed * 0.001>
        );

      #local Pos = Explosion_Center + Advect + Curl;
      #local Opacity = Smoke_Opacity * pow(1.0 - Age01, 0.45);

      SmokePuff(Pos, BaseR, Opacity, PuffSeed)
    #end

    #local I = I + 1;
  #end
#end
