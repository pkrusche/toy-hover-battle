#!/usr/bin/env bash
set -euo pipefail

asset_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDER_ROOT="$(cd "$asset_dir/.." && pwd)"
source "$RENDER_ROOT/scripts/render-lib.sh"

frames_dir="${FRAMES_DIR:-frames}"
output_image="${EXPLOSION_OUTPUT:-$frames_dir/explosion_sheet_iso_10x6.png}"
columns="${EXPLOSION_COLUMNS:-10}"
frame_count="${EXPLOSION_FRAMES:-60}"
width="${EXPLOSION_WIDTH:-256}"
height="${EXPLOSION_HEIGHT:-256}"
prefix="${EXPLOSION_PREFIX:-explosion_iso}"

explosion_seed="${EXPLOSION_SEED:-1701}"
smoke_seed="${EXPLOSION_SMOKE_SEED:-24011}"
fire_count="${EXPLOSION_FIRE_COUNT:-280}"
smoke_count="${EXPLOSION_SMOKE_COUNT:-70}"
smoke_enable="${EXPLOSION_SMOKE_ENABLE:-1}"
use_media_core="${EXPLOSION_USE_MEDIA_CORE:-0}"
preview_ground="${EXPLOSION_PREVIEW_GROUND:-0}"
view_height="${EXPLOSION_VIEW_HEIGHT:-14.00}"

aa_threshold="${EXPLOSION_AA_THRESHOLD:-0.12}"
aa_depth="${EXPLOSION_AA_DEPTH:-3}"
quality="${EXPLOSION_QUALITY:-9}"
clock_start="${EXPLOSION_CLOCK_START:-0.02}"
clock_end="${EXPLOSION_CLOCK_END:-0.72}"

validate_positive_int "EXPLOSION_FRAMES" "$frame_count"
validate_positive_int "EXPLOSION_COLUMNS" "$columns"

frames_path="$(repo_path "$frames_dir")"
output_path="$(repo_path "$output_image")"

expected_width=$((width * columns))
expected_height=$((height * ((frame_count + columns - 1) / columns)))
if render_output_up_to_date "$output_path" "$expected_width" "$expected_height"; then
  echo "$output_path is already ${expected_width}x${expected_height}; skipping render"
  exit 0
fi

mkdir -p "$frames_path" "$(dirname "$output_path")"
rm -f "$frames_path/${prefix}_"*.png "$frames_path/${prefix}_row_"*.png "$output_path"

export FRAMES_DIR="$frames_dir"
export EXPLOSION_FRAMES="$frame_count"
export EXPLOSION_WIDTH="$width"
export EXPLOSION_HEIGHT="$height"
export EXPLOSION_PREFIX="$prefix"
export EXPLOSION_SEED="$explosion_seed"
export EXPLOSION_SMOKE_SEED="$smoke_seed"
export EXPLOSION_FIRE_COUNT="$fire_count"
export EXPLOSION_SMOKE_COUNT="$smoke_count"
export EXPLOSION_SMOKE_ENABLE="$smoke_enable"
export EXPLOSION_USE_MEDIA_CORE="$use_media_core"
export EXPLOSION_PREVIEW_GROUND="$preview_ground"
export EXPLOSION_VIEW_HEIGHT="$view_height"
export EXPLOSION_AA_THRESHOLD="$aa_threshold"
export EXPLOSION_AA_DEPTH="$aa_depth"
export EXPLOSION_QUALITY="$quality"
export EXPLOSION_CLOCK_START="$clock_start"
export EXPLOSION_CLOCK_END="$clock_end"

render_job \
  FRAMES_DIR \
  EXPLOSION_FRAMES \
  EXPLOSION_WIDTH \
  EXPLOSION_HEIGHT \
  EXPLOSION_PREFIX \
  EXPLOSION_SEED \
  EXPLOSION_SMOKE_SEED \
  EXPLOSION_FIRE_COUNT \
  EXPLOSION_SMOKE_COUNT \
  EXPLOSION_SMOKE_ENABLE \
  EXPLOSION_USE_MEDIA_CORE \
  EXPLOSION_PREVIEW_GROUND \
  EXPLOSION_VIEW_HEIGHT \
  EXPLOSION_AA_THRESHOLD \
  EXPLOSION_AA_DEPTH \
  EXPLOSION_QUALITY \
  EXPLOSION_CLOCK_START \
  EXPLOSION_CLOCK_END <<'RENDER_SCRIPT'
set -euo pipefail

mkdir -p "$FRAMES_DIR"

render_explosion_frame() {
  local i="$1"
  local frame
  local clock_value

  frame=$(printf "%03d" "$i")

  if (( EXPLOSION_FRAMES == 1 )); then
    clock_value=$(awk "BEGIN { printf \"%.8f\", ($EXPLOSION_CLOCK_START + $EXPLOSION_CLOCK_END) / 2 }")
  else
    clock_value=$(awk "BEGIN { printf \"%.8f\", $EXPLOSION_CLOCK_START + ($EXPLOSION_CLOCK_END - $EXPLOSION_CLOCK_START) * $i / ($EXPLOSION_FRAMES - 1) }")
  fi

  povray +I"explosion/explosion.pov" \
    +K"$clock_value" \
    "Declare=Explosion_Seed=$EXPLOSION_SEED" \
    "Declare=Smoke_Seed=$EXPLOSION_SMOKE_SEED" \
    "Declare=Fire_Count=$EXPLOSION_FIRE_COUNT" \
    "Declare=Smoke_Count=$EXPLOSION_SMOKE_COUNT" \
    "Declare=Smoke_Enable=$EXPLOSION_SMOKE_ENABLE" \
    "Declare=Use_Media_Core=$EXPLOSION_USE_MEDIA_CORE" \
    "Declare=Preview_Ground=$EXPLOSION_PREVIEW_GROUND" \
    "Declare=ViewHeight=$EXPLOSION_VIEW_HEIGHT" \
    +O"${FRAMES_DIR}/${EXPLOSION_PREFIX}_${frame}" \
    +FN8 +UA +W"$EXPLOSION_WIDTH" +H"$EXPLOSION_HEIGHT" +Q"$EXPLOSION_QUALITY" \
    +AM2 +A"$EXPLOSION_AA_THRESHOLD" +R"$EXPLOSION_AA_DEPTH" -J \
    File_Gamma=sRGB
}

render_frames_parallel "$EXPLOSION_FRAMES" render_explosion_frame
RENDER_SCRIPT

frame_files=()
for ((i = 0; i < frame_count; i++)); do
  frame="$(printf "%03d" "$i")"
  frame_files+=("$frames_path/${prefix}_${frame}.png")
done

assemble_sprite_sheet "$output_path" "$columns" "${frame_files[@]}"
rm -f "${frame_files[@]}"
