#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/../.." && pwd)"
cd "$repo_root"

# A configured roster overrides TERRA_ROVER_COUNT, so clear it to guarantee
# that this launcher starts the full supported fleet of default Terra bodies.
unset ZORVANE_ROBOT_CONFIG
export TERRA_ROVER_COUNT=32

exec cargo run -p zorvane
