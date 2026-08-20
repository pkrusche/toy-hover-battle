This repository uses Jujutsu (`jj`) with a Git backend.

Versioning practice:
- Create a new JJ change for each larger logical change.
- Attach a clear commit description before considering the change complete.
- Keep commit descriptions specific to the user-visible or developer-visible outcome.

Repository layout:
- Keep one top-level folder per asset, such as `explosion/`, `rocks/`, and `ship/`.
- Each asset folder owns its POV-Ray scene file and `render.sh` entry point.
- Keep generated render output in `frames/`; it is ignored by Git.

Rendering workflow:
- Use the top-level `Makefile` as the main entry point.
- `make render` should render all assets.
- Individual asset targets should delegate to that asset's `render.sh`.
- Share common rendering behavior through `scripts/render-lib.sh`.
- Do not duplicate Docker setup, backend detection, or sprite-sheet assembly in asset scripts.
- Render scripts should work from the repository root so Docker and local POV-Ray paths behave the same way.
- Rendering should prefer Docker when the Docker CLI and daemon are available.
- If Docker is unavailable, rendering should fall back to local `povray`.

Installing into the game:
- `make install` copies generated output from `frames/` into the game's `assets/`.
- Only generated files are installed; hand-authored assets are never touched.
- `src/assets.rs` hardcodes each sprite sheet's frame layout, so a sheet whose
  dimensions differ from the installed asset is rejected unless `--force` is passed.
- Keep the installed filenames in sync with the `include_bytes!` paths in `src/assets.rs`.
