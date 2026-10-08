#!/usr/bin/env bash
# Capture town, tile, and NEXT screenshots plus a short driving GIF.
# Software Vulkan on Xvfb. Writes docs/assets/{town,tiles,next}.png and drive.gif.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ROOT}/target/debug/zorvane"
OUT="${ROOT}/docs/assets"
FRAMES="${OUT}/.drive-frames"

export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export VK_DRIVER_FILES="${VK_DRIVER_FILES:-${VK_ICD_FILENAMES}}"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"

window_id() {
  local id
  id="$(xwininfo -root -tree | awk '/Bevy|zorvane|Zorvane/ {print $1; exit}')"
  if [[ -z "${id}" ]]; then
    id="$(xwininfo -root -tree | awk 'NR > 1 && $1 ~ /^0x/ {print $1; exit}')"
  fi
  [[ -n "${id}" ]] || return 1
  printf '%s\n' "${id}"
}

grab() {
  local dest=$1
  local wid
  if wid="$(window_id)"; then
    import -window "${wid}" "png:${dest}"
  else
    import -window root "png:${dest}"
  fi
}

wait_window() {
  local pid=$1
  local i
  for i in $(seq 1 150); do
    if window_id >/dev/null; then
      echo "window after ${i}s"
      return 0
    fi
    if ! kill -0 "${pid}" 2>/dev/null; then
      echo "simulator exited before a window" >&2
      return 1
    fi
    sleep 1
  done
  echo "timed out waiting for a window" >&2
  xwininfo -root -tree >&2 || true
  return 1
}

sharpness() {
  convert "$1" -format '%[fx:standard_deviation]' info:
}

settle_shot() {
  local dest=$1
  local pid=$2
  local try score
  for try in 1 2 3 4 5 6 7 8; do
    sleep 10
    grab "${dest}"
    score="$(sharpness "${dest}" || echo 0)"
    echo "shot ${try} deviation ${score}"
    awk -v s="${score}" 'BEGIN { exit !(s+0 > 0.08) }' && return 0
    if ! kill -0 "${pid}" 2>/dev/null; then
      return 1
    fi
  done
  return 0
}

compress_png() {
  local src=$1
  local tmp
  tmp="$(mktemp --suffix=.png)"
  convert "${src}" -resize '960x540>' "${tmp}"
  if pngquant --quality=60-82 --force --output "${src}" "${tmp}"; then
    rm -f "${tmp}"
  else
    mv "${tmp}" "${src}"
  fi
}

session_still() {
  local name=$1
  shift
  env "$@" "${BIN}" >"/tmp/zorvane-${name}.log" 2>&1 &
  local pid=$!
  wait_window "${pid}"
  settle_shot "/tmp/zorvane-${name}.raw.png" "${pid}"
  kill "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true
}

session_drive() {
  mkdir -p "${FRAMES}"
  TERRA_NEXT=1 TERRA_ZENOH=0 "${BIN}" >/tmp/zorvane-drive.log 2>&1 &
  local pid=$!
  wait_window "${pid}"
  sleep 18
  local wid
  wid="$(window_id)"
  xdotool windowfocus --sync "${wid}" || true
  xdotool keydown --window "${wid}" w || xdotool keydown w
  local i n
  for i in $(seq 1 12); do
    printf -v n '%02d' "${i}"
    grab "${FRAMES}/frame_${n}.png"
    sleep 0.5
  done
  xdotool keyup --window "${wid}" w || xdotool keyup w
  kill "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true
}

if [[ "${1:-}" == "--session-still" ]]; then
  shift
  session_still "$@"
  exit 0
fi

if [[ "${1:-}" == "--session-drive" ]]; then
  session_drive
  exit 0
fi

if [[ ! -x "${BIN}" ]]; then
  echo "missing ${BIN}; build with: cargo build -p zorvane" >&2
  exit 1
fi

mkdir -p "${OUT}"

run_still() {
  local name=$1
  shift
  echo "=== still ${name} ==="
  xvfb-run -a -s "-screen 0 1400x900x24" "${BASH_SOURCE[0]}" --session-still "${name}" "$@"
  compress_png "/tmp/zorvane-${name}.raw.png"
  mv "/tmp/zorvane-${name}.raw.png" "${OUT}/${name}.png"
  echo "wrote ${OUT}/${name}.png"
}

run_still town TERRA_ZENOH=0
run_still tiles TERRA_TILES=1 TERRA_TILES_FETCH=0 TERRA_ZENOH=0
run_still next TERRA_NEXT=1 TERRA_ZENOH=0

echo "=== drive gif ==="
rm -rf "${FRAMES}"
xvfb-run -a -s "-screen 0 1400x900x24" "${BASH_SOURCE[0]}" --session-drive
ffmpeg -y -framerate 8 -i "${FRAMES}/frame_%02d.png" \
  -vf "scale=640:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=48[p];[s1][p]paletteuse" \
  "${OUT}/drive.gif"
gifsicle -O3 --lossy=60 -o "${OUT}/drive.gif" "${OUT}/drive.gif"
rm -rf "${FRAMES}"
ls -lh "${OUT}/town.png" "${OUT}/tiles.png" "${OUT}/next.png" "${OUT}/drive.gif"
