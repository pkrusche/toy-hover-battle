#!/usr/bin/env bash
set -euo pipefail

asset_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$asset_dir/.." && pwd)"

if ! command -v uv >/dev/null 2>&1; then
  echo "uv is required to run sfx/generate.py locally" >&2
  exit 1
fi

uv_args=(run --project "$asset_dir" --locked)
[[ -n "${SFX_PYTHON:-}" ]] && uv_args+=(--python "$SFX_PYTHON")

(cd "$repo_root" && uv "${uv_args[@]}" sfx/generate.py "$@")
