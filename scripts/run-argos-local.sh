#!/bin/sh
# NEXT on a local Zenoh router for ARGOS; never joins the hosted fleet.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
unset TERRA_ZENOH_CONFIG TERRA_ZENOH_CONNECT TERRA_ZENOH_USER TERRA_ZENOH_PASSWORD TERRA_ZENOH_TLS_ROOT_CA
export TERRA_ZENOH=1
export TERRA_ZENOH_LISTEN=tcp/127.0.0.1:7447
export TERRA_ZENOH_PREFIX=terra/rover
export ZORVANE_MENU=0
# Operators connect straight into the match; the intro cutscene is for players.
export ZORVANE_CUTSCENE=0
export TERRA_ROVER_COUNT=1
exec cargo run --locked --manifest-path "$root/Cargo.toml" -p zorvane
