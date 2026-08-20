#!/usr/bin/env bash
set -euo pipefail

asset_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDER_ROOT="$(cd "$asset_dir/.." && pwd)"
source "$RENDER_ROOT/scripts/render-lib.sh"

frames_dir="${FRAMES_DIR:-frames}"
output_image="${SHIP_OUTPUT:-$frames_dir/ship_strip.png}"
red_row="${SHIP_RED_ROW:-$frames_dir/ship_red_row.png}"
blue_row="${SHIP_BLUE_ROW:-$frames_dir/ship_blue_row.png}"
frame_count="${SHIP_FRAMES:-128}"
width="${SHIP_WIDTH:-200}"
height="${SHIP_HEIGHT:-200}"

validate_positive_int "SHIP_FRAMES" "$frame_count"

frames_path="$(repo_path "$frames_dir")"
output_path="$(repo_path "$output_image")"
red_row_path="$(repo_path "$red_row")"
blue_row_path="$(repo_path "$blue_row")"

expected_width=$((width * frame_count))
expected_height=$((height * 2))
if render_output_up_to_date "$output_path" "$expected_width" "$expected_height"; then
  echo "$output_path is already ${expected_width}x${expected_height}; skipping render"
  exit 0
fi

mkdir -p "$frames_path" "$(dirname "$output_path")" "$(dirname "$red_row_path")" "$(dirname "$blue_row_path")"
rm -f "$frames_path"/ship_red_*.png "$frames_path"/ship_blue_*.png "$red_row_path" "$blue_row_path" "$output_path"

export FRAMES_DIR="$frames_dir"
export SHIP_FRAMES="$frame_count"
export SHIP_WIDTH="$width"
export SHIP_HEIGHT="$height"

render_job FRAMES_DIR SHIP_FRAMES SHIP_WIDTH SHIP_HEIGHT <<'RENDER_SCRIPT'
set -euo pipefail

mkdir -p "$FRAMES_DIR"

render_ship_frame() {
  local task="$1"
  local i=$((task % SHIP_FRAMES))
  local accent_name
  local accent_red
  local accent_green
  local accent_blue
  local angle
  local frame

  if (( task < SHIP_FRAMES )); then
    accent_name=red
    accent_red=0.86
    accent_green=0.24
    accent_blue=0.20
  else
    accent_name=blue
    accent_red=0.20
    accent_green=0.24
    accent_blue=0.86
  fi

  angle=$(awk "BEGIN { printf \"%.4f\", $i * (360 / $SHIP_FRAMES) }")
  frame=$(printf "%03d" "$i")

  povray +I"ship/ship.pov" \
    "Declare=YawAngle=$angle" \
    "Declare=AccentRed=$accent_red" \
    "Declare=AccentGreen=$accent_green" \
    "Declare=AccentBlue=$accent_blue" \
    +O"${FRAMES_DIR}/ship_${accent_name}_${frame}" \
    +FN8 +UA +W"$SHIP_WIDTH" +H"$SHIP_HEIGHT" +Q9 \
    +AM2 +A0.2 +R3 -J \
    File_Gamma=sRGB
}

render_frames_parallel "$((SHIP_FRAMES * 2))" render_ship_frame
RENDER_SCRIPT

red_files=()
blue_files=()
for ((i = 0; i < frame_count; i++)); do
  frame="$(printf "%03d" "$i")"
  red_files+=("$frames_path/ship_red_${frame}.png")
  blue_files+=("$frames_path/ship_blue_${frame}.png")
done

append_images_horizontal "$red_row_path" "${red_files[@]}"
append_images_horizontal "$blue_row_path" "${blue_files[@]}"
append_images_vertical "$output_path" "$red_row_path" "$blue_row_path"
rm -f "${red_files[@]}" "${blue_files[@]}" "$red_row_path" "$blue_row_path"
