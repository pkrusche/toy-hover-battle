#!/usr/bin/env bash

: "${ASSET_IMAGE:=${POVRAY_IMAGE:-toy-assets}}"
: "${POVRAY_IMAGE:=$ASSET_IMAGE}"

if [[ -z "${RENDER_ROOT:-}" ]]; then
  render_lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  RENDER_ROOT="$(cd "$render_lib_dir/.." && pwd)"
fi

repo_path() {
  local path="$1"

  case "$path" in
    /*) printf '%s\n' "$path" ;;
    *) printf '%s/%s\n' "$RENDER_ROOT" "$path" ;;
  esac
}

require_command() {
  local command_name="$1"
  local message="$2"

  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "$message" >&2
    exit 1
  fi
}

validate_positive_int() {
  local name="$1"
  local value="$2"

  if ! [[ "$value" =~ ^[0-9]+$ ]] || (( value < 1 )); then
    echo "$name must be a positive integer" >&2
    exit 1
  fi
}

docker_available() {
  command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1
}

render_backend() {
  if docker_available; then
    printf 'docker\n'
  else
    printf 'local\n'
  fi
}

ensure_renderer() {
  local backend="$1"

  if [[ "$backend" == "docker" ]]; then
    ensure_container_image
  else
    require_command povray "POV-Ray (povray) is required when Docker is unavailable"
  fi
}

ensure_container_image() {
  if ! docker image inspect "$ASSET_IMAGE" >/dev/null 2>&1; then
    docker build -t "$ASSET_IMAGE" "$RENDER_ROOT"
  fi
}

docker_has_nvidia_runtime() {
  docker info --format '{{json .Runtimes}}' 2>/dev/null | grep -q '"nvidia"'
}

run_asset_container() {
  local docker_args=(--rm -v "$RENDER_ROOT:/work" -w /work)
  if docker_has_nvidia_runtime; then
    docker_args+=(--gpus all)
  fi
  docker run "${docker_args[@]}" "$@"
}

render_job() {
  local script
  script="$(cat)"
  script="$(cat <<'RENDER_JOB_PRELUDE'
set -euo pipefail

povray() {
  local povray_args=(+WT1 "$@")

  if [[ "${RENDER_VERBOSE:-0}" == "1" ]]; then
    command povray "${povray_args[@]}"
    return
  fi

  local log_file
  log_file="$(mktemp "${TMPDIR:-/tmp}/povray-render.XXXXXX.log")"

  if command povray "${povray_args[@]}" -D -V >"$log_file" 2>&1; then
    rm -f "$log_file"
    return 0
  else
    local status="$?"
    echo "POV-Ray failed with exit code $status" >&2
    echo "Command: povray ${povray_args[*]}" >&2
    sed -n '1,160p' "$log_file" >&2
    rm -f "$log_file"
    return "$status"
  fi
}

available_cpu_count() {
  local count=""

  if command -v nproc >/dev/null 2>&1; then
    count="$(nproc)"
  elif command -v getconf >/dev/null 2>&1; then
    count="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
  elif command -v sysctl >/dev/null 2>&1; then
    count="$(sysctl -n hw.logicalcpu 2>/dev/null || true)"
  fi

  if ! [[ "$count" =~ ^[0-9]+$ ]] || (( count < 1 )); then
    count=1
  fi

  printf '%s\n' "$count"
}

render_frames_parallel() {
  local task_count="$1"
  local render_frame="$2"
  local worker_count="${RENDER_JOBS:-$(available_cpu_count)}"

  if ! [[ "$task_count" =~ ^[0-9]+$ ]] || (( task_count < 1 )); then
    echo "frame task count must be a positive integer" >&2
    return 1
  fi

  if ! [[ "$worker_count" =~ ^[0-9]+$ ]] || (( worker_count < 1 )); then
    echo "RENDER_JOBS must be a positive integer" >&2
    return 1
  fi

  if (( worker_count > task_count )); then
    worker_count="$task_count"
  fi

  local worker_label="workers"
  if (( worker_count == 1 )); then
    worker_label="worker"
  fi
  echo "Rendering $task_count frame jobs with $worker_count $worker_label"

  local worker
  local pids=()
  for ((worker = 0; worker < worker_count; worker++)); do
    (
      local task
      for ((task = worker; task < task_count; task += worker_count)); do
        "$render_frame" "$task"
      done
    ) &
    pids+=("$!")
  done

  local status=0
  local pid
  for pid in "${pids[@]}"; do
    if ! wait "$pid"; then
      status=1
    fi
  done

  return "$status"
}
RENDER_JOB_PRELUDE
printf '%s\n' "$script"
)"

  local backend
  backend="$(render_backend)"
  ensure_renderer "$backend"
  echo "Rendering with $backend backend"

  local docker_env=()
  local local_env=()
  local name

  for name in "$@"; do
    if [[ -z "${!name+x}" ]]; then
      echo "Render variable $name is not set" >&2
      exit 1
    fi

    docker_env+=("-e" "$name=${!name}")
    local_env+=("$name=${!name}")
  done

  for name in RENDER_JOBS RENDER_VERBOSE; do
    if [[ -n "${!name+x}" ]]; then
      docker_env+=("-e" "$name=${!name}")
      local_env+=("$name=${!name}")
    fi
  done

  if [[ "$backend" == "docker" ]]; then
    run_asset_container \
      "${docker_env[@]}" \
      "$ASSET_IMAGE" \
      bash -lc "$script"
  else
    (cd "$RENDER_ROOT" && env "${local_env[@]}" bash -lc "$script")
  fi
}

imagemagick_convert() {
  MAGICK_CONFIGURE_PATH="$RENDER_ROOT/scripts/imagemagick-policy" convert "$@"
}

imagemagick_identify() {
  MAGICK_CONFIGURE_PATH="$RENDER_ROOT/scripts/imagemagick-policy" identify "$@"
}

# Whether $path already exists and its pixel dimensions equal
# $expected_width x $expected_height, so asset scripts can skip re-rendering
# when a matching output is already in frames/.
render_output_up_to_date() {
  local path="$1"
  local expected_width="$2"
  local expected_height="$3"

  [[ -f "$path" ]] || return 1
  command -v identify >/dev/null 2>&1 || return 1

  local actual
  actual="$(imagemagick_identify -format '%w %h' "${path}[0]" 2>/dev/null)" || return 1
  [[ "$actual" == "$expected_width $expected_height" ]]
}

assemble_sprite_sheet() {
  local output="$1"
  local columns="$2"
  shift 2

  local frames=("$@")
  validate_positive_int "columns" "$columns"
  require_command convert "ImageMagick (convert) is required to combine rendered frames"

  if (( ${#frames[@]} == 0 )); then
    echo "No frames supplied for $output" >&2
    exit 1
  fi

  mkdir -p "$(dirname "$output")"
  local base="${output%.*}"
  rm -f "${base}_row_"*.png "$output"

  local row_files=()
  local row=0
  local start

  for ((start = 0; start < ${#frames[@]}; start += columns)); do
    local row_images=("${frames[@]:start:columns}")
    local row_file
    row_file="$(printf "%s_row_%02d.png" "$base" "$row")"
    imagemagick_convert "${row_images[@]}" +append "$row_file"
    row_files+=("$row_file")
    row=$((row + 1))
  done

  imagemagick_convert "${row_files[@]}" -append "$output"
  rm -f "${row_files[@]}"
  echo "Wrote $output"
}

append_images_horizontal() {
  local output="$1"
  shift

  require_command convert "ImageMagick (convert) is required to combine rendered frames"
  mkdir -p "$(dirname "$output")"
  rm -f "$output"
  imagemagick_convert "$@" +append "$output"
  echo "Wrote $output"
}

append_images_vertical() {
  local output="$1"
  shift

  require_command convert "ImageMagick (convert) is required to combine rendered frames"
  mkdir -p "$(dirname "$output")"
  rm -f "$output"
  imagemagick_convert "$@" -append "$output"
  echo "Wrote $output"
}
