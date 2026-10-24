# Shared Terra runtime

Every production rover delegates estimator, command admission, control, mapping,
GPS/search and reconnect policy to `terra-runtime` from the pinned sibling Terra
checkout. `terra-source.json` identifies the entire source revision. CI fetches
that revision using `.github/scripts/checkout-terra.sh`; it must match the phone's
runtime source metadata. All twelve Terra dependencies resolve from that checkout.

The simulator supplies synthetic IMU/VIO and geographic/detector measurements,
steps the runtime at 100 Hz, and applies returned motor effort through physics.
Ground clearance uses the mount/body calibration, independent of terrain elevation.
Navigation uses estimated motion; simulator truth generates sensor evidence and
supports evaluation. There is no alternate production autonomy scheduler.

The default actuation profile is velocity feedback. `TERRA_ACTUATION_MODE=direct_effort`
selects the same normalized manual profile as the phone; autonomous levels still
use feedback. Plant feed-forward calibration reflects simulator motor force/drag.
Managed no-spin constraints, source/settings admission and retained-mission policy
are shared. Subscription refresh does not replace drive authority; a new transport
connection does. Recording health is independent of the router connection.

Final verification: workspace suite (116 passed, zero failed), actual simulator
sensor-adapter replay in both profiles, native build, plain/browser WebAssembly
checks and browser WebAssembly build. All three live TCP fixtures were explicitly
run: bound drive with RGB/depth telemetry, independent fleet resizing, and the
phone client's motor-force drive/disconnect path passed. Obsolete live fixtures
now use bound teleop and the production runtime; reverse commands respect the
shared default reverse limit. The two GPU tests remain skipped in workspace runs.
Terra's native/WebAssembly replay compares 200 ticks with exact discrete statuses
and zero measured numeric error. Physical rover rollout remains separate.
