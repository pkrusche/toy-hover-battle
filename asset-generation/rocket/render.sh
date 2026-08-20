#!/usr/bin/env bash
set -euo pipefail

asset_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDER_ROOT="$(cd "$asset_dir/.." && pwd)"
source "$RENDER_ROOT/scripts/render-lib.sh"

frames_dir="${FRAMES_DIR:-frames}"
output_image="${ROCKET_OUTPUT:-$frames_dir/rocket_strip.png}"
frame_count="${ROCKET_FRAMES:-64}"
width="${ROCKET_WIDTH:-32}"
height="${ROCKET_HEIGHT:-32}"

validate_positive_int "ROCKET_FRAMES" "$frame_count"

frames_path="$(repo_path "$frames_dir")"
output_path="$(repo_path "$output_image")"

expected_width=$((width * frame_count))
if render_output_up_to_date "$output_path" "$expected_width" "$height"; then
  echo "$output_path is already ${expected_width}x${height}; skipping render"
  exit 0
fi

mkdir -p "$frames_path" "$(dirname "$output_path")"
rm -f "$frames_path"/rocket_yellow_*.png "$output_path"

export FRAMES_DIR="$frames_dir"
export ROCKET_FRAMES="$frame_count"
export ROCKET_WIDTH="$width"
export ROCKET_HEIGHT="$height"

render_job FRAMES_DIR ROCKET_FRAMES ROCKET_WIDTH ROCKET_HEIGHT <<'RENDER_SCRIPT'
set -euo pipefail

mkdir -p "$FRAMES_DIR"

render_rocket_frame() {
  local i="$1"
  local angle
  local frame

  angle=$(awk "BEGIN { printf \"%.4f\", $i * (360 / $ROCKET_FRAMES) }")
  frame=$(printf "%03d" "$i")

  povray +I"rocket/rocket.pov" \
    "Declare=YawAngle=$angle" \
    "Declare=AccentRed=0.92" \
    "Declare=AccentGreen=0.75" \
    "Declare=AccentBlue=0.05" \
    +O"${FRAMES_DIR}/rocket_yellow_${frame}" \
    +FN8 +UA +W"$ROCKET_WIDTH" +H"$ROCKET_HEIGHT" +Q9 \
    +AM2 +A0.2 +R3 -J \
    File_Gamma=sRGB
}

render_frames_parallel "$ROCKET_FRAMES" render_rocket_frame
RENDER_SCRIPT

yellow_files=()
for ((i = 0; i < frame_count; i++)); do
  frame="$(printf "%03d" "$i")"
  yellow_files+=("$frames_path/rocket_yellow_${frame}.png")
done

append_images_horizontal "$output_path" "${yellow_files[@]}"
rm -f "${yellow_files[@]}"
