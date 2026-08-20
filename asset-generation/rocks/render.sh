#!/usr/bin/env bash
set -euo pipefail

asset_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDER_ROOT="$(cd "$asset_dir/.." && pwd)"
source "$RENDER_ROOT/scripts/render-lib.sh"

frames_dir="${FRAMES_DIR:-frames}"
output_image="${ROCK_OUTPUT:-$frames_dir/rock_sheet_iso_4x4.png}"
columns="${ROCK_COLUMNS:-4}"
variants="${ROCK_VARIANTS:-16}"
seed_start="${ROCK_SEED_START:-1201}"
width="${ROCK_WIDTH:-256}"
height="${ROCK_HEIGHT:-256}"
use_outline="${ROCK_USE_OUTLINE:-0}"
mask_mode="${ROCK_MASK_MODE:-0}"
color_variation="${ROCK_COLOR_VARIATION:-1}"
vertical_scale="${ROCK_VERTICAL_SCALE:-0.65}"
floor_cut="${ROCK_FLOOR_CUT:--0.38}"
prefix="${ROCK_PREFIX:-rock_iso}"

validate_positive_int "ROCK_VARIANTS" "$variants"
validate_positive_int "ROCK_COLUMNS" "$columns"

frames_path="$(repo_path "$frames_dir")"
output_path="$(repo_path "$output_image")"

expected_width=$((width * columns))
expected_height=$((height * ((variants + columns - 1) / columns)))
if render_output_up_to_date "$output_path" "$expected_width" "$expected_height"; then
  echo "$output_path is already ${expected_width}x${expected_height}; skipping render"
  exit 0
fi

mkdir -p "$frames_path" "$(dirname "$output_path")"
rm -f "$frames_path/${prefix}_seed"*.png "$frames_path/${prefix}_row_"*.png "$output_path"

export FRAMES_DIR="$frames_dir"
export ROCK_VARIANTS="$variants"
export ROCK_SEED_START="$seed_start"
export ROCK_WIDTH="$width"
export ROCK_HEIGHT="$height"
export ROCK_USE_OUTLINE="$use_outline"
export ROCK_MASK_MODE="$mask_mode"
export ROCK_COLOR_VARIATION="$color_variation"
export ROCK_VERTICAL_SCALE="$vertical_scale"
export ROCK_FLOOR_CUT="$floor_cut"
export ROCK_PREFIX="$prefix"

render_job \
  FRAMES_DIR \
  ROCK_VARIANTS \
  ROCK_SEED_START \
  ROCK_WIDTH \
  ROCK_HEIGHT \
  ROCK_USE_OUTLINE \
  ROCK_MASK_MODE \
  ROCK_COLOR_VARIATION \
  ROCK_VERTICAL_SCALE \
  ROCK_FLOOR_CUT \
  ROCK_PREFIX <<'RENDER_SCRIPT'
set -euo pipefail

mkdir -p "$FRAMES_DIR"

render_rock_frame() {
  local i="$1"
  local seed=$((ROCK_SEED_START + i))

  povray +I"rocks/rock.pov" \
    "Declare=SeedValue=$seed" \
    "Declare=Use_Outline=$ROCK_USE_OUTLINE" \
    "Declare=Mask_Mode=$ROCK_MASK_MODE" \
    "Declare=Color_Variation=$ROCK_COLOR_VARIATION" \
    "Declare=Vertical_Scale=$ROCK_VERTICAL_SCALE" \
    "Declare=Floor_Cut=$ROCK_FLOOR_CUT" \
    +O"${FRAMES_DIR}/${ROCK_PREFIX}_seed${seed}" \
    +FN8 +UA +W"$ROCK_WIDTH" +H"$ROCK_HEIGHT" +Q9 \
    +AM2 +A0.08 +R3 -J \
    File_Gamma=sRGB
}

render_frames_parallel "$ROCK_VARIANTS" render_rock_frame
RENDER_SCRIPT

frame_files=()
for ((i = 0; i < variants; i++)); do
  seed=$((seed_start + i))
  frame_files+=("$frames_path/${prefix}_seed${seed}.png")
done

assemble_sprite_sheet "$output_path" "$columns" "${frame_files[@]}"
rm -f "${frame_files[@]}"
