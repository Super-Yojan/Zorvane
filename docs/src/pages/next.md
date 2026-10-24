---
layout: ../layouts/DocPage.astro
title: The game
description: NEXT 2026 on Field 3, West Campus. Collect tennis balls, load them at your goal, and dump them into your alliance's buckets.
---

The field follows the *NEXT Competition 2026 Rules* (August 7, 2026). Red's Base is behind the south goal, Blue's behind the north.

<figure>
<img src="/Zorvane/media/field3.jpg" alt="The NEXT field from above the west touchline: the fenced pitch, light towers, scoreboard, Base tents, and the buckets on each half." width="1600" height="1220">
<figcaption>Field 3 from above the west touchline. Rover 0 starts on the south goal line, facing north.</figcaption>
</figure>

| Rule | In the simulator |
| --- | --- |
| 2.1 Regulation soccer field | 100 × 64 m pitch with FIFA markings and the yellow 30-yard lines |
| 2.2 Base tents | Red Base behind the south end line, Blue Base behind the north |
| 3.1 99 balls in each penalty box | 99 per box, spread by a seed, clear of the goal area |
| 3.2 10 orange and 10 blue buckets | Orange, Red's targets, on the north half. Blue, Blue's targets, on the south half. 4 per half on 1 m stands |
| 4 Scoring | A ball dumped into your ground bucket scores 1, or 2 within 30 m of the opponent's Base |
| 4.3 Out of bounds, 4.4 tagging, 5.6 knocked buckets | In the cutscene, not enforced |

Launching is not modeled, so the raised buckets (3 and 6 points) are out of reach. Their rims are about 1.34 m up, above the dumper.

## One scoring run

1. **Collect.** Drive over balls. CrabTrapBot's intake holds 8; at capacity the flaps close and further balls stay on the field.
2. **Return.** Drive back to the goal line you started on. Within 1 m of it, collected balls become *waiting*.
3. **Load.** Publish `next/load`, or click **LOAD 1 INTO DUMPER**, once per ball. The dumper holds 2.
4. **Park.** Drive into the other half and park with the outlet over one of your alliance's ground buckets. `next/status` reports `can_dump` once it is.
5. **Dump.** Publish `next/dump`, or click **DUMP**. The balls fall for real, and only the ones that land in the bucket score.

`next/status` reports the stage as `SEARCHING`, `CARRYING BALLS`, `FULL: RETURN TO GOAL` or `WAITING FOR OPERATOR`. `next/score` carries both alliances' points.

## Where the field is

The pitch sits on [Field #3](https://www.openstreetmap.org/way/233194680), centred at `38.833116675, -77.323160625` with its long axis at 327.04° from north. Simulated GPS and the `localization` report use that placement, so outdoor GPS navigation runs on the simulated field the way it would on the real one. Placement data is © OpenStreetMap contributors, under the [ODbL](https://www.openstreetmap.org/copyright).

## What the camera sees

Buckets are Home Depot orange (`#F96302`) and blue (`#1D5DB2`). Balls are optic yellow with a white seam. The turf, fence, light towers, scoreboard and tents match the competition-day field. `TERRA_NEXT_LIGHTING` switches between noon, overcast and evening, for testing a detector in different light.
