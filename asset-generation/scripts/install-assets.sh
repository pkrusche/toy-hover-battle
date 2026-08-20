#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDER_ROOT="$(cd "$script_dir/.." && pwd)"
source "$RENDER_ROOT/scripts/render-lib.sh"

usage() {
  cat <<'USAGE'
Usage: install-assets.sh [--force] [asset...]

Copy generated render and sound output into the game's assets directory.

Assets: explosion, rocks, ship, rocket, sfx (default: all)

Options:
  --force   install sprite sheets whose dimensions differ from the installed
            asset. The game hardcodes sheet geometry, so a size change is
            treated as a mistake unless it is requested explicitly.

Environment:
  GAME_ASSETS_DIR   destination directory (default: ../assets)
  FRAMES_DIR        generated output directory (default: frames)
  EXPLOSION_OUTPUT, ROCK_OUTPUT, SHIP_OUTPUT, ROCKET_OUTPUT, SFX_OUTPUT_DIR
                    source paths, matching the render and sfx scripts
USAGE
}

force=0
selected=()

while (( $# > 0 )); do
  case "$1" in
    --force) force=1 ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "unknown option: $1" >&2; usage >&2; exit 1 ;;
    *) selected+=("$1") ;;
  esac
  shift
done

frames_dir="${FRAMES_DIR:-frames}"
dest_dir="$(repo_path "${GAME_ASSETS_DIR:-../assets}")"
# Canonicalize so the reported paths stay readable (the default is ../assets).
if [[ -d "$dest_dir" ]]; then
  dest_dir="$(cd "$dest_dir" && pwd)"
fi

# Destination names are the paths src/assets.rs includes at build time; only the
# source side is configurable.
sheet_assets=(explosion rocks ship rocket)
sheet_source_explosion="${EXPLOSION_OUTPUT:-$frames_dir/explosion_sheet_iso_10x6.png}"
sheet_source_rocks="${ROCK_OUTPUT:-$frames_dir/rock_sheet_iso_4x4.png}"
sheet_source_ship="${SHIP_OUTPUT:-$frames_dir/ship_strip.png}"
sheet_source_rocket="${ROCKET_OUTPUT:-$frames_dir/rocket_strip.png}"
sheet_name_explosion="explosion_sheet_iso_10x6.png"
sheet_name_rocks="rock_sheet_iso_4x4.png"
sheet_name_ship="ship_strip.png"
sheet_name_rocket="rocket_strip.png"
sfx_source_dir="${SFX_OUTPUT_DIR:-$frames_dir/sfx}"

if (( ${#selected[@]} == 0 )); then
  selected=("${sheet_assets[@]}" sfx)
fi

for asset in "${selected[@]}"; do
  case "$asset" in
    explosion|rocks|ship|rocket|sfx) ;;
    *) echo "unknown asset: $asset" >&2; usage >&2; exit 1 ;;
  esac
done

selects() {
  local wanted="$1" asset
  for asset in "${selected[@]}"; do
    [[ "$asset" == "$wanted" ]] && return 0
  done
  return 1
}

# Reads a big-endian PNG header without depending on ImageMagick, so the guard
# below also works on a machine that only renders through Docker.
png_dimensions() {
  local file="$1"
  local -a bytes
  # shellcheck disable=SC2207
  bytes=($(od -An -tu1 -N24 "$file" 2>/dev/null))
  (( ${#bytes[@]} == 24 )) || return 1
  (( bytes[0] == 137 && bytes[1] == 80 && bytes[2] == 78 && bytes[3] == 71 )) || return 1
  printf '%dx%d\n' \
    "$(( (bytes[16] << 24) | (bytes[17] << 16) | (bytes[18] << 8) | bytes[19] ))" \
    "$(( (bytes[20] << 24) | (bytes[21] << 16) | (bytes[22] << 8) | bytes[23] ))"
}

is_wav() {
  local file="$1"
  local -a bytes
  # shellcheck disable=SC2207
  bytes=($(od -An -tu1 -N12 "$file" 2>/dev/null))
  (( ${#bytes[@]} == 12 )) || return 1
  (( bytes[0] == 82 && bytes[1] == 73 && bytes[2] == 70 && bytes[3] == 70 )) || return 1
  (( bytes[8] == 87 && bytes[9] == 65 && bytes[10] == 86 && bytes[11] == 69 ))
}

errors=()
plan_sources=()
plan_destinations=()
plan_notes=()

plan_copy() {
  plan_sources+=("$1")
  plan_destinations+=("$2")
  plan_notes+=("$3")
}

for asset in "${sheet_assets[@]}"; do
  selects "$asset" || continue

  source_ref="sheet_source_$asset"
  name_ref="sheet_name_$asset"
  source_path="$(repo_path "${!source_ref}")"
  dest_path="$dest_dir/${!name_ref}"

  if [[ ! -f "$source_path" ]]; then
    errors+=("missing $source_path — run 'make $asset' first")
    continue
  fi

  if ! source_dims="$(png_dimensions "$source_path")"; then
    errors+=("$source_path is not a readable PNG")
    continue
  fi

  if [[ -f "$dest_path" ]]; then
    if cmp -s "$source_path" "$dest_path"; then
      echo "unchanged $dest_path ($source_dims)"
      continue
    fi
    dest_dims="$(png_dimensions "$dest_path" || true)"
    if [[ -n "$dest_dims" && "$dest_dims" != "$source_dims" ]] && (( force == 0 )); then
      errors+=("$asset sheet is $source_dims but the installed asset is $dest_dims; the game hardcodes sheet geometry — pass --force if the change is intended")
      continue
    fi
    plan_copy "$source_path" "$dest_path" "$source_dims, was ${dest_dims:-unknown}"
  else
    plan_copy "$source_path" "$dest_path" "$source_dims, new"
  fi
done

if selects sfx; then
  sfx_path="$(repo_path "$sfx_source_dir")"
  if [[ ! -d "$sfx_path" ]]; then
    errors+=("missing $sfx_path — run 'make sfx' first")
  else
    shopt -s nullglob
    wav_files=("$sfx_path"/*.wav)
    shopt -u nullglob
    if (( ${#wav_files[@]} == 0 )); then
      errors+=("no WAV files in $sfx_path — run 'make sfx' first")
    fi
    for wav in "${wav_files[@]}"; do
      dest_path="$dest_dir/sfx/$(basename "$wav")"
      if ! is_wav "$wav"; then
        errors+=("$wav is not a readable WAV")
        continue
      fi
      if [[ -f "$dest_path" ]] && cmp -s "$wav" "$dest_path"; then
        echo "unchanged $dest_path"
        continue
      fi
      plan_copy "$wav" "$dest_path" "$( [[ -f "$dest_path" ]] && echo replaced || echo new )"
    done
  fi
fi

if (( ${#errors[@]} > 0 )); then
  printf 'error: %s\n' "${errors[@]}" >&2
  exit 1
fi

if (( ${#plan_sources[@]} == 0 )); then
  echo "Game assets already up to date"
  exit 0
fi

for ((i = 0; i < ${#plan_sources[@]}; i++)); do
  source_path="${plan_sources[i]}"
  dest_path="${plan_destinations[i]}"
  mkdir -p "$(dirname "$dest_path")"
  temp_path="$(mktemp "$(dirname "$dest_path")/.$(basename "$dest_path").XXXXXX")"
  cp "$source_path" "$temp_path"
  chmod 644 "$temp_path"
  mv -f "$temp_path" "$dest_path"
  echo "Installed $dest_path (${plan_notes[i]})"
done
