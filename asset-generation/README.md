# Toy Assets

POV-Ray source assets and render scripts for Toy Hover Battle sprites, plus
seeded Stable Audio sound-effect generation.

## Repository Layout

- `explosion/` - explosion scene source and render script.
- `rocks/` - rock scene source and render script.
- `ship/` - ship scene source and render script.
- `rocket/` - rocket scene source and render script.
- `sfx/` - Stable Audio prompts, generator, and pinned Python dependencies.
- `scripts/render-lib.sh` - shared render helpers used by asset scripts.
- `scripts/install-assets.sh` - copies generated output into the game's assets.
- `frames/` - generated render output. This directory is ignored by Git.

Each asset folder owns its `.pov` scene file and its `render.sh` entry point.

## Requirements

Rendering requires:

- Bash
- ImageMagick `convert`
- Either Docker or a local `povray` install

The render scripts prefer Docker when the Docker CLI and daemon are available.
If Docker is not available, they run `povray` directly on the host.

Sound-effect generation requires [`uv`](https://docs.astral.sh/uv/) and access to
[`stabilityai/stable-audio-open-1.0`](https://huggingface.co/stabilityai/stable-audio-open-1.0).
Accept the model's Stability AI license on Hugging Face, then download it to a
local cache with the `hf` CLI (via `uvx`, no local install required):

```sh
uvx hf download stabilityai/stable-audio-open-1.0 --local-dir /path/to/stable-audio-open-1.0
```

Point `SFX_MODEL_ID` at that local directory to generate without downloading
model weights at runtime:

```bash
SFX_MODEL_ID=/path/to/stable-audio-open-1.0 make sfx
```

## Rendering

Render every asset:

```sh
make render
```

Render visuals and generate missing sound effects:

```sh
make all
```

Render one asset:

```sh
make explosion
make rocks
make ship
make rocket
```

Remove generated output:

```sh
make clean
```

## Updating the Game Assets

Rendering and sound generation write to the ignored `frames/` directory. Copy
that output into the game's `assets/` directory with:

```sh
make install
```

Regenerate everything and then install it in one step:

```sh
make update-assets
```

Only the generated files are touched: the four sprite sheets and the WAV files
under `assets/sfx/`. Hand-authored assets such as `assets/background.frag` and
the `assets/icon_*.rgba` icons are left alone. Files that already match are
reported as unchanged and not rewritten, and each file is replaced atomically,
so an interrupted install cannot leave a truncated asset behind.

`src/assets.rs` hardcodes each sheet's frame layout (for example, 128 ship
frames of 200x200 and a 10x6 explosion grid), so installing a sheet whose
dimensions differ from the installed asset fails with an error. Confirm the
render settings match the game's expectations, then repeat with `--force`:

```sh
make install INSTALL_ARGS="--force"
```

`INSTALL_ARGS` also selects individual assets by name (`explosion`, `rocks`,
`ship`, `rocket`, `sfx`), which is useful after re-rendering just one of them:

```sh
make install INSTALL_ARGS="rocks"
```

The destination defaults to `../assets` and can be overridden with
`GAME_ASSETS_DIR`. The script can also be run directly:

```sh
./scripts/install-assets.sh --help
```

The asset scripts can also be run directly:

```sh
./explosion/render.sh
./rocks/render.sh
./ship/render.sh
./rocket/render.sh
```

## Sound Effects

Generate all missing WAV files:

```sh
make sfx
```

`sfx/generate.sh` runs locally through `uv`, which manages the pinned
dependencies in `sfx/pyproject.toml` and `sfx/uv.lock`. Just run the entry
point; `uv` creates and syncs an isolated environment automatically. The
generator selects CUDA, Apple MPS, or CPU; CPU inference is supported but can
take a long time, especially for all ten effects.

```sh
./sfx/generate.sh
```

`SFX_PYTHON` can name a different Python interpreter or version for `uv` to
use. Direct invocation via `uv run` is also supported when run from the
repository root:

```sh
uv run --project sfx sfx/generate.py
```

Select effects by manifest name, validate without downloading model weights, or
replace existing output:

```sh
./sfx/generate.sh menu_move menu_confirm
./sfx/generate.sh --dry-run
./sfx/generate.sh rocket_warning --force
```

Output is 44.1 kHz stereo PCM16 WAV under ignored `frames/sfx/`. Existing files
are skipped unless `--force` is supplied. `make clean` removes generated visual
and audio output.

Configuration can be overridden with `SFX_OUTPUT_DIR`, `SFX_MODEL_ID`,
`SFX_DEVICE`, `SFX_INFERENCE_STEPS`, `SFX_GUIDANCE_SCALE`, and `HF_HOME`.
Runtime credentials for a non-bundled model belong only in `HF_TOKEN`; do not
put them in these settings or in the repository.

## Common Render Options

Set environment variables before running `make` or an asset script to override
defaults. Examples:

```sh
EXPLOSION_FRAMES=12 EXPLOSION_COLUMNS=6 make explosion
ROCK_VARIANTS=8 ROCK_COLUMNS=4 make rocks
SHIP_FRAMES=64 make ship
ROCKET_FRAMES=64 ROCKET_WIDTH=32 ROCKET_HEIGHT=32 make rocket
```

Useful output overrides:

- `FRAMES_DIR` - directory for generated intermediate frames and, by default,
  assembled sprite sheets. Overriding it therefore keeps rendering and
  `make install` in sync.
- `RENDER_JOBS` - maximum number of concurrent frame renders; defaults to the
  CPUs available to the local process or Docker container.
- `EXPLOSION_OUTPUT` - explosion sprite sheet path.
- `ROCK_OUTPUT` - rock sprite sheet path.
- `SHIP_OUTPUT` - ship sprite sheet path.
- `ROCKET_OUTPUT` - rocket sprite strip path.

Asset-specific defaults live in each asset's `render.sh`.

Each frame is rendered by a single-threaded POV-Ray process. The render scripts
run up to `RENDER_JOBS` frame processes at once, so `make render` uses the
available CPUs without each POV-Ray process also competing for every core. Set
`RENDER_JOBS=1` to render frames sequentially.
