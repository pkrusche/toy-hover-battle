# Security policy

Toy Hover Battle is an offline, single-machine game: it opens no network
sockets, runs no servers, and reads no untrusted input beyond keyboard and
gamepad events. The realistic security surface is therefore small — but reports
are still welcome as Github issues.

## Known accepted risks

These are tracked in `deny.toml` and re-checked on every dependency bump:

- **RUSTSEC-2025-0035** — soundness issues in `macroquad` (pervasive mutable
  statics, use-after-free reachable from safe code). No fixed version exists
  and the entire game is built on macroquad, so the risk is accepted.
- **RUSTSEC-2026-0192** — `ttf-parser` is unmaintained. It arrives transitively
  through `fontdue` → `macroquad` and cannot be selected from here.

The macOS gamepad path in `src/pads.rs` is the project's only `unsafe` code. It
is compiled by CI on every push, but has no automated runtime coverage.

Released binaries are unsigned: Windows SmartScreen will warn on first run, and
macOS is not shipped as a binary at all for that reason.
