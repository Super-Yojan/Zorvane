#!/bin/sh
# Build/run with the adjacent Terra checkout instead of the release Git pin.
# The dedicated executable survives subsequent normal cargo builds.
set -eu
simulator_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
terra_root=$(CDPATH= cd -- "$simulator_root/../Terra" && pwd)
case "${1:-}" in
    ""|--build-only) ;;
    *) echo 'Usage: scripts/run-local-terra.sh [--build-only]' >&2; exit 2 ;;
esac
build_workspace=$(mktemp -d /tmp/zorvane-local-terra.XXXXXX)
trap 'rm -rf "$build_workspace"' EXIT HUP INT TERM
python3 - "$simulator_root" "$terra_root" "$build_workspace" <<'PY'
import pathlib,re,sys
simulator,terra,workspace=map(pathlib.Path,sys.argv[1:])
manifest=(simulator/'Cargo.toml').read_text()
def local_dependency(match):
    crate=match[1]
    path=terra/'crates'/crate
    if not (path/'Cargo.toml').is_file():
        raise SystemExit(f'Missing local crate: {path}')
    return f'{crate} = {{ path = "{path}" }}'
manifest,count=re.subn(r'(terra-[a-z]+) = \{ git = [^\n]+',local_dependency,manifest)
if count==0: raise SystemExit('No pinned Terra dependencies found')
(workspace/'Cargo.toml').write_text(manifest)
(workspace/'Cargo.lock').write_bytes((simulator/'Cargo.lock').read_bytes())
for directory in ('simulator','crates'):
    (workspace/directory).symlink_to(simulator/directory,target_is_directory=True)
PY
cargo build --offline --manifest-path "$build_workspace/Cargo.toml" --target-dir "$simulator_root/target" -p zorvane
mkdir -p "$simulator_root/target/local-terra-bin"
cp "$simulator_root/target/debug/zorvane" "$simulator_root/target/local-terra-bin/zorvane"
if [ "${1:-}" = --build-only ]; then
    echo "Built $simulator_root/target/local-terra-bin/zorvane"
    exit 0
fi
rm -rf "$build_workspace"
trap - EXIT HUP INT TERM
cd "$simulator_root"
export ZORVANE_ASSETS="$simulator_root/simulator/assets"
export TERRA_TILE_CACHE="${TERRA_TILE_CACHE:-$simulator_root/simulator/.cache/tiles}"
exec "$simulator_root/target/local-terra-bin/zorvane"
