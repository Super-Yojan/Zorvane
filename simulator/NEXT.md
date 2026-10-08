# NEXT competition practice

`TERRA_NEXT=1` loads a dedicated practice pitch for the NEXT competition instead of the town or the real-world tile patch. Zatara is the primary rover: spawn slot zero of the `terra-ground` body (Terra's chassis, `models/rover.glb`, and differential drive). It is not a second vehicle type. The scene is a clean approximation of a small-sided soccer field: an 18 m by 12 m pitch, perimeter boards, six tennis balls, and two open-front deposit buckets in the north penalty area. It is not an official field survey, and it does not score a match.

Zatara starts at the origin on the south half, facing the buckets (Bevy −Z). Balls rest around midfield. Drive into a ball to push it; once a ball’s center crosses a bucket mouth, a light pull draws it to the back wall so it stays deposited. The bucket opening is wider than the chassis, so Zatara can nose in and then reverse out. Boards keep balls on the pitch.

## Launch

From the Zorvane repository root:

```sh
TERRA_NEXT=1 TERRA_ZENOH=0 cargo run -p zorvane
```

`TERRA_ZENOH=0` leaves the primary rover on the keyboard. W and S drive forward and back, A and D turn, and Space commands a stop. With Zenoh left on, a remote client drives Zatara as rover 0. Zenoh keys stay `terra/rover/<id>/…`:

```sh
TERRA_NEXT=1 cargo run -p zorvane
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.8 --angular 0 --seconds 3
```

Headless smoke, no window and no Zenoh:

```sh
TERRA_NEXT=1 TERRA_HEADLESS=1 TERRA_ZENOH=0 ZORVANE_SMOKE=1 cargo run -p zorvane
```

`TERRA_NEXT=1` turns off the practice town and Terrarium tiles and sets the ground to a 32 m square. It also takes precedence over `TERRA_MISSION=1`. `TERRA_ROVER_COUNT` still changes the fleet; extra rovers use the usual spacing around Zatara. `ZORVANE_VEHICLE` still selects the body and defaults to `terra-ground`, which is what Zatara drives. `TERRA_HEADLESS=1` runs the same scene without a window.

Startup prints a `Terra NEXT:` line with the ball and bucket counts.
