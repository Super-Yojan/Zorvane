# NEXT competition practice

**TL;DR.** `TERRA_NEXT=1` loads a full soccer pitch. Zatara is rover 0 on `terra-ground`. Six tennis balls. Two buckets. No scoring.

![NEXT pitch](../docs/assets/next.png)

*105 m by 68 m. Zatara starts on the centre spot and faces the north goal.*

![Zatara driving](../docs/assets/drive.gif)

*Hold W, or send `cmd_vel`. The rover pushes balls. Buckets keep what crosses the mouth.*

Zatara starts at the origin, the centre spot, facing Bevy −Z (north).
Balls rest just north of that spot.
A ball past the mouth is pulled to the back wall.
The opening is wider than the chassis.
Boards keep balls on the pitch.

The pitch is the FIFA recommended international size, 105 m by 68 m, on a 120 m square. Centre circle, penalty areas, and goal areas use the Law 1 sizes. See [SCALE.md](../SCALE.md).

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

`TERRA_NEXT=1` turns off the practice town and Terrarium tiles and sets the ground to a 120 m square. It also takes precedence over `TERRA_MISSION=1`. `TERRA_ROVER_COUNT` still changes the fleet; extra rovers use the usual spacing around Zatara. `ZORVANE_VEHICLE` still selects the body and defaults to `terra-ground`, which is what Zatara drives. `TERRA_HEADLESS=1` runs the same scene without a window.

Startup prints a `Terra NEXT:` line with the ball and bucket counts.
