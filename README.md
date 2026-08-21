# Toy Hover Battle

[![CI](https://github.com/pkrusche/toy-hover-battle/actions/workflows/ci.yml/badge.svg)](https://github.com/pkrusche/toy-hover-battle/actions/workflows/ci.yml)

Small isometric battle game that was made for me to experiment with AI-based coding. This one / two-player arena game is written in Rust with [Macroquad](https://macroquad.rs/). Pilot a craft through a procedurally generated rock field face a single opponent — human or AI.


> Created with the help of AI.
> The included sound effects were generated with [Stable Audio Open 1.0](https://huggingface.co/stabilityai/stable-audio-open-1.0). The model and its weights are not included and remain subject to the [Stability AI Community License](https://huggingface.co/stabilityai/stable-audio-open-1.0/blob/main/LICENSE.md).
> Graphics are procedurally generated at runtime or rendered from the included [POV-Ray](https://www.povray.org/) scene sources.
>
> See [`asset-generation/`](asset-generation/README.md) for the source assets and scripts that produce the sprite sheets and sound effects.

## Gameplay

![screenshot](assets/thb.webp)

## Controls

| Action | Player 1 | Player 2 |
| --- | --- | --- |
| Throttle / Brake | `W` / `S` | `Arrow Up` / `Arrow Down` |
| Turn | `A` / `D` | `Arrow Left` / `Arrow Right` |
| Strafe | `R` / `T` | `,` / `.` |
| Fire | `Left Shift` | `Right Shift` / `/` |
| Rocket | `Q` | `Enter` |

Gamepads are also supported and auto-assigned to a free player slot on connect:

| Action | Gamepad |
| --- | --- |
| Throttle / Brake | D-Pad Up / Down |
| Turn | D-Pad Left / Right |
| Strafe | Right Shoulder / Left Shoulder |
| Fire | A |
| Rocket | B |

Menus reuse the D-Pad to navigate and A / B to confirm / back.

Other keys:

- `Enter` — confirm menu selection
- `Escape` / `Backspace` — back out of a menu
- `H` — controls/help screen (from the start menu)
- `F3` — toggle the AI debug overlay (shown over AI-controlled viewports during a match)

In match setup, use `Arrow Up` / `Arrow Down` to move between rows and `Arrow
Left` / `Arrow Right` to change the focused row — each player's controller
(Human or an AI difficulty) and the match's Speed (Slow / Normal / Fast),
which scales the whole match's pace to further tune the challenge.

## Download

Prebuilt Linux and Windows binaries are attached to every tagged release on the
[releases page](https://github.com/pkrusche/toy-hover-battle/releases/latest). Each archive
unpacks into a single folder holding the executable plus `LICENSE`, this
`README.md`, and `THIRD-PARTY-LICENSES.md`.

- **Linux** (`toy-hover-battle-linux-x86_64.tar.gz`):

  ```sh
  tar -xzf toy-hover-battle-linux-x86_64.tar.gz
  ./toy-hover-battle-linux-x86_64/toy-hover-battle
  ```

  The binary is dynamically linked against the system's X11/GL and ALSA
  libraries; install your distribution's equivalents of `libasound2` and the
  usual OpenGL/X11 runtime if the game does not start.

- **Windows** (`toy-hover-battle-windows-x86_64.zip`): unpack and run
  `toy-hover-battle.exe`. The binary is unsigned, so SmartScreen may ask for a
  confirmation the first time.

- **macOS**: no prebuilt binary — build from source as described below.

## Building from source

Requires the pinned Rust 1.97.1 [toolchain](https://rustup.rs/), installed
automatically by `rustup` from `rust-toolchain.toml`.

```sh
git clone https://github.com/pkrusche/toy-hover-battle.git
cd toy-hover-battle
cargo run --release
```

`cargo run` also works for faster iteration during development (the crate sets `opt-level = 3` for dependencies even in debug builds, so it stays playable).

### Platform notes

- **Linux**: Macroquad needs the usual X11/GL dev libraries (and ALSA for audio) available on your system.
- **macOS**: build from source — releases carry no macOS binary, because an unsigned and unnotarized download is quarantined by Gatekeeper. Builds are covered by CI on Apple silicon. Gamepad support goes through a direct `GameController` framework binding rather than `gilrs`.
- **Windows**: nothing special beyond a working Rust toolchain.

## Development

```sh
cargo test    # unit tests for AI intercept math, line-of-fire, avoidance steering
cargo clippy
cargo fmt
```

Development-agent conventions are documented in `AGENTS.md`.

## License

The entire repository, including the Rust source, build scripts, and bundled
assets, is available under the Creative Commons Attribution-NonCommercial 4.0
International license (CC BY-NC 4.0); see [`LICENSE`](LICENSE). See
[`assets/LICENSE.md`](assets/LICENSE.md) for asset provenance and the separate
terms governing the Stable Audio model, which is not included in this
repository.

CC BY-NC 4.0 makes this project **source-available, not open source**: the
non-commercial restriction fails the OSI definition, so GitHub shows no license
badge, distribution packagers (AUR, Homebrew, Flathub, itch's commercial paths)
will treat it as non-free, and anything commercial needs separate permission.
This is deliberate — it is a toy project, not a product — but worth knowing
before you build on it.

The Rust dependencies the binary links are under their own (mostly MIT /
Apache-2.0) licenses. Release archives ship a generated `THIRD-PARTY-LICENSES.md`
with the full texts; regenerate it locally with
[cargo-about](https://github.com/EmbarkStudios/cargo-about):

```sh
cargo install --locked --features cli cargo-about
cargo about generate --locked about.hbs --output-file THIRD-PARTY-LICENSES.md
```
