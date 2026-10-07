#!/usr/bin/env bash
# Boot Zorvane without a window. ZORVANE_SMOKE=1 exits after the schedule starts.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

export TERRA_HEADLESS="${TERRA_HEADLESS:-1}"
export TERRA_ZENOH="${TERRA_ZENOH:-0}"
export ZORVANE_SMOKE="${ZORVANE_SMOKE:-1}"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"

if [[ -z "${VK_ICD_FILENAMES:-}" || ! -f "${VK_ICD_FILENAMES}" ]]; then
  for candidate in \
    /usr/share/vulkan/icd.d/lvp_icd.x86_64.json \
    /usr/share/vulkan/icd.d/lvp_icd.json
  do
    if [[ -f "${candidate}" ]]; then
      export VK_ICD_FILENAMES="${candidate}"
      break
    fi
  done
fi
if [[ -n "${VK_ICD_FILENAMES:-}" ]]; then
  export VK_DRIVER_FILES="${VK_DRIVER_FILES:-${VK_ICD_FILENAMES}}"
fi

exec cargo run -p zorvane -- "$@"
