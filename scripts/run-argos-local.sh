#!/bin/sh
# Local simulation using the adjacent Terra runtime; never joins the hosted fleet.
set -eu
simulator_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
unset TERRA_ZENOH_CONFIG TERRA_ZENOH_CONNECT TERRA_ZENOH_USER TERRA_ZENOH_PASSWORD TERRA_ZENOH_TLS_ROOT_CA
export TERRA_ZENOH=1
export TERRA_ZENOH_LISTEN=tcp/127.0.0.1:7447
export TERRA_ZENOH_PREFIX=terra/rover
# The NEXT competition on Field 3, West Campus, with simulated GPS there.
export TERRA_TILES=0
export TERRA_NEXT=1
export TERRA_NEXT_GMU=0
export TERRA_MISSION=0
export ZORVANE_MENU=0
# Operators connect straight into the match; the intro cutscene is for players.
export ZORVANE_CUTSCENE=0
export TERRA_ROVER_COUNT=1
exec "$simulator_root/scripts/run-local-terra.sh"
