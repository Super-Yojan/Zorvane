"""Fetch the mapped surroundings of the NEXT field into `mason_osm.json`.

The NEXT 2026 rules place the competition on Field 3, West Campus, George
Mason University: OpenStreetMap way 233194680 ("Field #3", turf). Features
within 420 m are converted into the NEXT pitch frame (metres, +x across the
pitch to the east touchline, +y toward the north goal, origin at the pitch
centre) for `build_next_assets.py`.

    python3 simulator/tools/blender/fetch_osm.py

Data © OpenStreetMap contributors, ODbL 1.0.
"""

import json
import math
import sys
import urllib.parse
import urllib.request
from pathlib import Path

# Field #3 centroid and its long-axis bearing toward the "north" goal, from
# the mapped outline of way 233194680. Keep in step with
# `simulator/src/mason_field.rs`.
LATITUDE = 38.833116675
LONGITUDE = -77.323160625
HEADING = 327.04
FIELD_WAY = "way/233194680"
RADIUS_M = 420
OVERPASS = "https://overpass-api.de/api/interpreter"
KEEP = (
    "building", "leisure", "amenity", "natural", "landuse", "highway", "barrier",
    "surface", "sport", "name", "building:levels", "height", "lanes", "width",
    "service", "footway", "power", "man_made",
)


def pitch_frame(lat, lon):
    metres_per_degree = 6_371_000 * math.pi / 180
    east = (lon - LONGITUDE) * metres_per_degree * math.cos(math.radians(LATITUDE))
    north = (lat - LATITUDE) * metres_per_degree
    h = math.radians(HEADING)
    return [round(east * math.cos(h) - north * math.sin(h), 2), round(east * math.sin(h) + north * math.cos(h), 2)]


def main():
    query = (
        f"[out:json][timeout:90];("
        f'way(around:{RADIUS_M},{LATITUDE},{LONGITUDE})[~"^(building|leisure|amenity|natural|landuse|highway|barrier|surface)$"~"."];'
        f'node(around:{RADIUS_M},{LATITUDE},{LONGITUDE})[~"^(natural|power|highway|man_made)$"~"."];'
        f");out body geom;"
    )
    request = urllib.request.Request(
        OVERPASS,
        data=urllib.parse.urlencode({"data": query}).encode(),
        headers={"User-Agent": "Zorvane-sim-asset-build/1.0", "Accept": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=180) as response:
        data = json.load(response)
    features = []
    for element in data["elements"]:
        tags = {k: v for k, v in element.get("tags", {}).items() if k in KEEP}
        if not tags:
            continue
        if element["type"] == "way" and "geometry" in element:
            features.append({
                "osm": f"way/{element['id']}",
                "tags": tags,
                "closed": element["nodes"][0] == element["nodes"][-1],
                "points": [pitch_frame(p["lat"], p["lon"]) for p in element["geometry"]],
            })
        elif element["type"] == "node":
            features.append({"osm": f"node/{element['id']}", "tags": tags, "point": pitch_frame(element["lat"], element["lon"])})
    out = {
        "source": "OpenStreetMap via Overpass API, retrieved " + data["osm3s"]["timestamp_osm_base"],
        "license": "ODbL 1.0, (c) OpenStreetMap contributors, https://www.openstreetmap.org/copyright",
        "field": FIELD_WAY,
        "frame": (
            f"Metres in the NEXT pitch frame: +x east across the pitch, +y toward the north goal "
            f"(pitch heading {HEADING} deg clockwise from true north), origin at the pitch centre {LATITUDE}, {LONGITUDE}."
        ),
        "features": features,
    }
    target = Path(__file__).resolve().parent / "mason_osm.json"
    target.write_text(json.dumps(out, separators=(",", ":")))
    print(f"{len(features)} features -> {target}")


if __name__ == "__main__":
    sys.exit(main())
