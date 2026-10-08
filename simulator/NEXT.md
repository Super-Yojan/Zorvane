# NEXT competition practice

**TL;DR.** `TERRA_NEXT=1` loads a practice pitch. Zatara is rover 0 on `terra-ground`. Six tennis balls. Two buckets. No scoring.

![NEXT practice pitch](../docs/assets/next.png)

*18 m by 12 m pitch on a 32 m square. Zatara faces the buckets.*

![Zatara driving](../docs/assets/drive.gif)

*Hold W, or send `cmd_vel`. The rover pushes balls. Buckets keep what crosses the mouth.*

Zatara starts at the origin, on the south half, facing Bevy −Z.
Balls rest around midfield.
A ball past the mouth is pulled to the back wall.
The opening is wider than the chassis.
Boards keep balls on the pitch.

This is a clean practice layout. It is not an official survey.

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
