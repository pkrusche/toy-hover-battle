# AGENTS.md

This repository uses Jujutsu (`jj`) with a Git backend.

Versioning practice:
- Create a new JJ change for each larger logical change.
- Attach a clear commit description before considering the change complete.
- Keep commit descriptions specific to the user-visible or developer-visible outcome.

Repository layout:
- The game itself lives in `src/`, `Cargo.toml`, and `assets/`.
- `asset-generation/` is a self-contained subproject that renders the game's sprite sheets and sound effects; see `asset-generation/AGENTS.md` for its conventions and `asset-generation/README.md` for usage.

Development:
- Build/run: `cargo run --release` (or plain `cargo run`; dev-profile deps still build at `opt-level = 3`).
- Before considering a change complete, run `cargo test`, `cargo clippy`, and `cargo fmt`.
- Unit tests live inline in their module (`#[test]` blocks in `src/*.rs`), not in a separate `tests/` directory — add new tests next to the code they cover.

Release process:
- `./make_release.sh VERSION` bumps the version, runs the full check suite, pushes `main`, and tags `vVERSION`, which triggers `.github/workflows/release.yml`.
- Release archives bundle `LICENSE`, `README.md`, and a generated `THIRD-PARTY-LICENSES.md` (cargo-about, configured in `about.toml`) alongside the binary.
