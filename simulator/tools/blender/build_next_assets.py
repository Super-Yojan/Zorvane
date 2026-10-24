"""Build the NEXT competition assets for Field 3, West Campus (GMU) with Blender.

Run headless from the Zorvane repository root:

    /Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup \
        --python simulator/tools/blender/build_next_assets.py -- --out simulator/assets

Add `--preview DIR` to also render reference images with Cycles.

Outputs, under `--out`:

- `models/homer_bucket.glb`, `models/blue_bucket.glb`: 5-gallon buckets in
  Home Depot orange (#F96302, with a generic white label patch where the
  store's logo is printed) and blue, sized from the NEXT bucket constants,
  origin at the centre of the base.
- `models/mason_surroundings.glb`: everything around the NEXT pitch: the
  competition-day furniture seen in the NEXT2026 footage (black chain-link
  perimeter fence, light towers, scoreboard, pop-up canopies, bench) and the
  campus beyond it, built from the mapped OpenStreetMap footprints in
  `mason_osm.json` (ODbL). The pitch, lines, boards, goals, balls and buckets
  stay authored in Rust. Objects named `solid:*` should get colliders.
- `textures/turf.png`, `textures/tennis_ball.png`: tileable turf for the pitch
  and an equirectangular optic-yellow felt map with the white seam.

Blender is Z-up; the glTF exporter writes Y-up, so Blender +X (east across the
pitch) is Bevy +X and Blender +Y (the north goal) is Bevy -Z, the NEXT frame.
Everything is deterministic: the same inputs build byte-identical geometry.
"""

import argparse
import json
import math
import random
import sys
from pathlib import Path

import bmesh
import bpy
import numpy as np
from mathutils import Vector
from mathutils.geometry import tessellate_polygon

HERE = Path(__file__).resolve().parent

# NEXT constants (simulator/src/next_competition.rs).
PITCH_HALF_WIDTH = 32.0
PITCH_HALF_LENGTH = 50.0
WORLD_HALF = 70.0
BUCKET_TOP_DIAMETER = 0.3048
BUCKET_BASE_DIAMETER = 0.3023
BUCKET_BODY_HEIGHT = 0.3399
BUCKET_WALL = 0.0025

# Home Depot "Homer" orange, #F96302. The NEXT field also uses blue buckets;
# their exact pigment is not published, so this is a mid hardware-store blue
# matched by eye to the NEXT2026 footage.
HOMER_ORANGE = "#F96302"
BUCKET_BLUE = "#1D5DB2"
# ITF optic yellow felt, approximately #DFFF4F.
OPTIC_YELLOW = "#DFFF4F"
SEAM_WHITE = "#F4F4EC"

# Everything mapped beyond this radius is left out.
SITE_RADIUS = 380.0


def srgb_to_linear(hex_color):
    hex_color = hex_color.lstrip("#")
    rgb = [int(hex_color[i : i + 2], 16) / 255.0 for i in (0, 2, 4)]
    return [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in rgb]


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--preview", type=Path)
    return parser.parse_args(argv)


def reset_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.unit_settings.system = "METRIC"
    return scene


# ---------------------------------------------------------------- textures


def value_noise(rng, size, cells):
    """Tileable value noise in [0, 1], `cells` lattice points per side."""
    lattice = rng.random((cells, cells))
    coords = np.arange(size) * cells / size
    i0 = np.floor(coords).astype(int)
    f = coords - i0
    f = f * f * (3 - 2 * f)
    i1 = (i0 + 1) % cells
    a = lattice[np.ix_(i0, i0)]
    b = lattice[np.ix_(i0, i1)]
    c = lattice[np.ix_(i1, i0)]
    d = lattice[np.ix_(i1, i1)]
    fx = f[None, :]
    fy = f[:, None]
    return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy


def fractal(rng, size, base=4, octaves=6):
    total = np.zeros((size, size))
    weight = 0.0
    for octave in range(octaves):
        amplitude = 0.55**octave
        total += value_noise(rng, size, base * 2**octave) * amplitude
        weight += amplitude
    return total / weight


def save_png(path, rgb, name):
    """rgb: float array (h, w, 3) in sRGB [0, 1], row 0 at the top."""
    h, w, _ = rgb.shape
    rgba = np.ones((h, w, 4), dtype=np.float32)
    rgba[:, :, :3] = np.clip(rgb, 0, 1)
    image = bpy.data.images.new(name, width=w, height=h, alpha=False)
    image.colorspace_settings.name = "sRGB"
    image.pixels.foreach_set(rgba[::-1].reshape(-1))
    path.parent.mkdir(parents=True, exist_ok=True)
    image.filepath_raw = str(path)
    image.file_format = "PNG"
    image.save()
    return image


def hex_rgb(hex_color):
    hex_color = hex_color.lstrip("#")
    return np.array([int(hex_color[i : i + 2], 16) / 255.0 for i in (0, 2, 4)])


def turf_texture(path, size=1024, seed=7):
    """Artificial turf, tileable over 4 m: even colour, blade-scale speckle and
    faint tufting rows, as on the NEXT2026 field."""
    rng = np.random.default_rng(seed)
    patches = fractal(rng, size, base=4, octaves=3)
    blades = rng.random((size, size))
    rows = 0.5 + 0.5 * np.cos(np.arange(size) / size * 2 * np.pi * 42)[None, :]
    dark = hex_rgb("#35612A")
    light = hex_rgb("#5A8A3E")
    tone = 0.3 + 0.15 * patches + 0.45 * blades + 0.1 * rows
    rgb = dark[None, None] * (1 - tone[..., None]) + light[None, None] * tone[..., None]
    return save_png(path, rgb, "turf")


def grass_texture(path, size=1024, seed=11):
    """Natural grass for the surroundings: duller and patchier than the pitch."""
    rng = np.random.default_rng(seed)
    patches = fractal(rng, size, base=2, octaves=6)
    blades = rng.random((size, size))
    dry = hex_rgb("#6E7D45")
    green = hex_rgb("#4A7234")
    mix = np.clip(0.25 + patches * 0.7, 0, 1)
    rgb = dry * (1 - mix[..., None]) + green * mix[..., None]
    rgb *= (0.9 + 0.2 * blades)[..., None]
    return save_png(path, rgb, "grass")


def speckle_texture(path, base_hex, spread, seed, name, size=512):
    rng = np.random.default_rng(seed)
    noise = fractal(rng, size, base=8, octaves=4) * 0.6 + rng.random((size, size)) * 0.4
    rgb = hex_rgb(base_hex)[None, None] * (1 - spread + 2 * spread * noise[..., None])
    return save_png(path, rgb, name)


def brick_texture(path, size=512, seed=5):
    """Running-bond brick, 8 courses per texture repeat."""
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:size, 0:size] / size
    course = np.floor(y * 8)
    shifted = x * 4 + 0.5 * (course % 2)
    mortar = (np.abs((y * 8) % 1 - 0.5) > 0.44) | (np.abs(shifted % 1 - 0.5) > 0.47)
    brick_id = (course * 13 + np.floor(shifted)).astype(int)
    tint = rng.random(brick_id.max() + 1)[brick_id]
    brick = hex_rgb("#8E4B37") * (0.92 + 0.12 * tint)[..., None]
    # Low mortar contrast: the sim samples without mipmaps, so a high-contrast
    # grid would shimmer at distance.
    rgb = np.where(mortar[..., None], hex_rgb("#9C6E5E"), brick)
    return save_png(path, rgb, "brick")


def tennis_seam(n=2000, b=0.32):
    """Unit-sphere points on the classic tennis-ball seam (a + b = 1)."""
    a = 1 - b
    t = np.linspace(0, 2 * np.pi, n, endpoint=False)
    p = np.stack(
        [a * np.cos(t) + b * np.cos(3 * t), a * np.sin(t) - b * np.sin(3 * t), 2 * np.sqrt(a * b) * np.sin(2 * t)],
        axis=1,
    )
    return p / np.linalg.norm(p, axis=1, keepdims=True)


def tennis_ball_texture(path, width=2048, height=1024, seed=3):
    """Equirectangular felt: optic yellow with fibre noise and the white seam.

    u runs around the vertical axis, v from the top pole, matching Bevy's
    `Sphere::mesh().uv(..)` layout. The seam's absolute rotation is arbitrary,
    like a real ball's.
    """
    rng = np.random.default_rng(seed)
    v, u = np.mgrid[0:height, 0:width]
    lon = (u + 0.5) / width * 2 * np.pi
    lat = np.pi / 2 - (v + 0.5) / height * np.pi
    dirs = np.stack([np.cos(lat) * np.cos(lon), np.sin(lat), np.cos(lat) * np.sin(lon)], axis=-1).reshape(-1, 3)
    seam = tennis_seam()
    # Seam half-width ~ 3.5 mm on a 33.5 mm radius ball.
    closest = np.full(dirs.shape[0], -1.0)
    for chunk in np.array_split(np.arange(seam.shape[0]), 20):
        closest = np.maximum(closest, (dirs @ seam[chunk].T).max(axis=1))
    angle = np.arccos(np.clip(closest, -1, 1)).reshape(height, width)
    half_width = 0.0035 / 0.0335
    seam_mask = np.clip((half_width - angle) / (half_width * 0.35) + 0.5, 0, 1)
    fuzz = rng.random((height, width))
    fibres = value_noise(rng, max(width, height), 256)[:height, :width]
    felt = hex_rgb(OPTIC_YELLOW)[None, None] * (0.86 + 0.1 * fibres[..., None] + 0.06 * fuzz[..., None])
    # Felt darkens slightly into the seam groove.
    groove = np.clip((angle - half_width) / (half_width * 0.8), 0, 1)
    felt *= (0.9 + 0.1 * groove)[..., None]
    rgb = felt * (1 - seam_mask[..., None]) + hex_rgb(SEAM_WHITE)[None, None] * seam_mask[..., None]
    return save_png(path, rgb, "tennis_ball")


# ---------------------------------------------------------------- materials


def material(name, color=None, roughness=0.6, metallic=0.0, image=None, alpha=None):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    if color is not None:
        bsdf.inputs["Base Color"].default_value = (*srgb_to_linear(color), 1.0)
    if image is not None:
        tex = mat.node_tree.nodes.new("ShaderNodeTexImage")
        tex.image = image
        mat.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    if alpha is not None:
        bsdf.inputs["Alpha"].default_value = alpha
        mat.surface_render_method = "BLENDED"
    return mat


# ---------------------------------------------------------------- geometry helpers


def new_object(name, mesh, mat, collection):
    obj = bpy.data.objects.new(name, mesh)
    if mat is not None:
        obj.data.materials.append(mat)
    collection.objects.link(obj)
    return obj


def planar_uv(bm, scale):
    """World-space XY (or the face's dominant plane) to UV, `scale` metres per repeat."""
    uv = bm.loops.layers.uv.verify()
    for face in bm.faces:
        n = face.normal
        for loop in face.loops:
            co = loop.vert.co
            if abs(n.z) >= max(abs(n.x), abs(n.y)):
                loop[uv].uv = (co.x / scale, co.y / scale)
            elif abs(n.x) >= abs(n.y):
                loop[uv].uv = (co.y / scale, co.z / scale)
            else:
                loop[uv].uv = (co.x / scale, co.z / scale)


def polygon_area(points):
    return 0.5 * sum(x0 * y1 - x1 * y0 for (x0, y0), (x1, y1) in zip(points, points[1:] + points[:1]))


def clean_ring(points):
    ring = [tuple(p) for p in points]
    if len(ring) > 1 and ring[0] == ring[-1]:
        ring = ring[:-1]
    if polygon_area(ring) < 0:
        ring.reverse()
    return ring


def fill_up(bm, verts):
    """Triangulate a (possibly concave) horizontal ring, every face pointing up."""
    triangles = tessellate_polygon([[Vector(v.co) for v in verts]])
    for a, b, c in triangles:
        face = bm.faces.new((verts[a], verts[b], verts[c]))
        face.normal_update()
        if face.normal.z < 0:
            face.normal_flip()


def prism(name, ring, bottom, top, mat, collection, uv_scale=4.0, cap=True):
    """Extrude a footprint between two heights."""
    bm = bmesh.new()
    lower = [bm.verts.new((x, y, bottom)) for x, y in ring]
    upper = [bm.verts.new((x, y, top)) for x, y in ring]
    n = len(ring)
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((lower[i], lower[j], upper[j], upper[i]))
    if cap:
        fill_up(bm, upper)
    bm.normal_update()
    planar_uv(bm, uv_scale)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    return new_object(name, mesh, mat, collection)


def flat_patch(name, ring, height, mat, collection, uv_scale):
    bm = bmesh.new()
    verts = [bm.verts.new((x, y, height)) for x, y in ring]
    fill_up(bm, verts)
    bm.normal_update()
    planar_uv(bm, uv_scale)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    return new_object(name, mesh, mat, collection)


def strip(name, points, width, height, mat, collection, uv_scale=4.0):
    """A flat ribbon along a polyline, mitred at the joints."""
    pts = [np.array(p, dtype=float) for p in points]
    if len(pts) < 2:
        return None
    left, right = [], []
    for i, p in enumerate(pts):
        a = pts[max(i - 1, 0)]
        b = pts[min(i + 1, len(pts) - 1)]
        d = b - a
        length = np.linalg.norm(d)
        if length < 1e-6:
            continue
        normal = np.array([-d[1], d[0]]) / length
        left.append(p + normal * width / 2)
        right.append(p - normal * width / 2)
    if len(left) < 2:
        return None
    bm = bmesh.new()
    lv = [bm.verts.new((x, y, height)) for x, y in left]
    rv = [bm.verts.new((x, y, height)) for x, y in right]
    for i in range(len(lv) - 1):
        face = bm.faces.new((rv[i], rv[i + 1], lv[i + 1], lv[i]))
        face.normal_update()
        if face.normal.z < 0:
            face.normal_flip()
    bm.normal_update()
    planar_uv(bm, uv_scale)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    return new_object(name, mesh, mat, collection)


def point_in_ring(x, y, ring):
    inside = False
    for (x0, y0), (x1, y1) in zip(ring, ring[1:] + ring[:1]):
        if (y0 > y) != (y1 > y) and x < (x1 - x0) * (y - y0) / (y1 - y0) + x0:
            inside = not inside
    return inside


def clip_to_site(ring):
    return [p for p in ring if math.hypot(*p) < SITE_RADIUS * 1.6]


# ---------------------------------------------------------------- bucket


def bucket_patch(name, radius_at, a0, a1, z0, z1, mat, collection, lift=0.0009):
    """A curved strip on the bucket wall, standing just proud of it."""
    bm = bmesh.new()
    steps = max(2, int(abs(a1 - a0) / math.radians(4)))
    lower, upper = [], []
    for i in range(steps + 1):
        a = a0 + (a1 - a0) * i / steps
        for z, row in ((z0, lower), (z1, upper)):
            r = radius_at(z) + lift
            row.append(bm.verts.new((r * math.cos(a), r * math.sin(a), z)))
    for i in range(steps):
        bm.faces.new((lower[i], lower[i + 1], upper[i + 1], upper[i]))
    bm.normal_update()
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    return new_object(name, mesh, mat, collection)


def build_bucket(out, materials, body, filename, label):
    """5-gallon bucket: tapered body, rim lip, three stiffening ribs, base ring,
    wire bail with a plastic grip. Open top. Dimensions follow the NEXT
    constants so the visual matches the Rust collider. `label` adds a generic
    white printed panel (a bordered square and text lines), standing in for a
    store logo without reproducing it."""
    collection = bpy.data.collections.new(filename)
    bpy.context.scene.collection.children.link(collection)
    orange = body
    top_r = BUCKET_TOP_DIAMETER / 2
    base_r = BUCKET_BASE_DIAMETER / 2
    h = BUCKET_BODY_HEIGHT
    wall = BUCKET_WALL

    def radius_at(z):
        return base_r + (top_r - base_r) * z / h

    # Outer profile (r, z) from the base up, with ribs as small bumps, then the
    # inner wall back down. Lathed around Z.
    rib_heights = [h - 0.012, h - 0.032, h - 0.052]
    outer = [(0.0, 0.0), (base_r - 0.006, 0.0), (base_r, 0.006), (base_r + 0.002, 0.014), (base_r, 0.02)]
    z = 0.02
    for rib in sorted(rib_heights):
        outer += [(radius_at(rib - 0.004), rib - 0.004), (radius_at(rib) + 0.0035, rib - 0.002),
                  (radius_at(rib) + 0.0035, rib + 0.002), (radius_at(rib + 0.004), rib + 0.004)]
    outer += [(top_r + 0.004, h - 0.004), (top_r + 0.005, h), (top_r - wall, h)]
    inner = [(radius_at(h) - wall, h - 0.002), (base_r - wall, wall), (0.0, wall)]
    profile = outer + inner

    segments = 48
    bm = bmesh.new()
    rings = []
    for r, zz in profile:
        ring = []
        for s in range(segments):
            angle = s / segments * 2 * math.pi
            ring.append(bm.verts.new((r * math.cos(angle), r * math.sin(angle), zz)))
        rings.append(ring)
    for a, b in zip(rings, rings[1:]):
        for s in range(segments):
            t = (s + 1) % segments
            bm.faces.new((a[s], a[t], b[t], b[s]))
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-6)
    bm.normal_update()
    uv = bm.loops.layers.uv.verify()
    for face in bm.faces:
        for loop in face.loops:
            co = loop.vert.co
            loop[uv].uv = ((math.atan2(co.y, co.x) / (2 * math.pi)) % 1.0, co.z / h)
    mesh = bpy.data.meshes.new("bucket_body")
    bm.to_mesh(mesh)
    bm.free()
    body = new_object("bucket_body", mesh, orange, collection)
    for poly in body.data.polygons:
        poly.use_smooth = True

    # Handle lugs at opposite sides, then the wire bail lying folded to one side.
    lug_z = h - 0.03
    for side in (-1, 1):
        bpy.ops.mesh.primitive_cylinder_add(vertices=16, radius=0.011, depth=0.012,
                                            location=(side * (top_r + 0.004), 0, lug_z),
                                            rotation=(0, math.pi / 2, 0))
        lug = bpy.context.active_object
        lug.name = f"bucket_lug_{'east' if side > 0 else 'west'}"
        lug.data.materials.append(orange)
        for c in lug.users_collection:
            c.objects.unlink(lug)
        collection.objects.link(lug)

    curve = bpy.data.curves.new("bucket_bail", "CURVE")
    curve.dimensions = "3D"
    curve.bevel_depth = 0.0018
    curve.bevel_resolution = 3
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(2)
    reach = top_r + 0.004
    folded = math.radians(70)
    pts = [(-reach, 0, lug_z), (0, -math.sin(folded) * 0.17, lug_z + math.cos(folded) * 0.17), (reach, 0, lug_z)]
    for bp, co in zip(spline.bezier_points, pts):
        bp.co = co
        bp.handle_left_type = bp.handle_right_type = "AUTO"
    bail = bpy.data.objects.new("bucket_bail", curve)
    bail.data.materials.append(materials["steel"])
    collection.objects.link(bail)

    bpy.ops.mesh.primitive_cylinder_add(vertices=16, radius=0.011, depth=0.1,
                                        location=pts[1], rotation=(0, math.pi / 2, 0))
    grip = bpy.context.active_object
    grip.name = "bucket_grip"
    grip.data.materials.append(materials["black_plastic"])
    for c in grip.users_collection:
        c.objects.unlink(grip)
    collection.objects.link(grip)

    if label:
        white = materials["label_white"]
        front = -math.pi / 2  # Facing -Y.
        half = math.radians(24)
        z0, z1 = 0.07, 0.235
        stroke_a, stroke_z = math.radians(2.2), 0.007
        bucket_patch("label border bottom", radius_at, front - half, front + half, z0, z0 + stroke_z, white, collection)
        bucket_patch("label border top", radius_at, front - half, front + half, z1 - stroke_z, z1, white, collection)
        bucket_patch("label border left", radius_at, front - half, front - half + stroke_a, z0, z1, white, collection)
        bucket_patch("label border right", radius_at, front + half - stroke_a, front + half, z0, z1, white, collection)
        for i, (width, z) in enumerate(((0.75, 0.19), (0.9, 0.16), (0.6, 0.13), (0.8, 0.10))):
            bucket_patch(f"label line {i}", radius_at, front - half * width * 0.8, front + half * width * 0.8,
                         z, z + 0.014, white, collection)

    export(collection, out / "models" / filename)
    return collection


# ---------------------------------------------------------------- surroundings


ROAD_WIDTH = {
    "primary": 11.0, "primary_link": 6.0, "secondary": 9.0, "tertiary": 8.0,
    "unclassified": 6.5, "residential": 6.5, "service": 4.5, "turning_circle": 0.0,
    "footway": 2.0, "cycleway": 2.5, "path": 1.5, "pedestrian": 3.0,
}
BUILDING_HEIGHT = {
    "university": 13.0, "apartments": 15.0, "detached": 7.5, "house": 7.5,
    "semidetached_house": 7.5, "garage": 3.0, "yes": 6.0,
}


def building_height(tags):
    if "height" in tags:
        try:
            return float(tags["height"].split()[0])
        except ValueError:
            pass
    if "building:levels" in tags:
        try:
            return float(tags["building:levels"]) * 3.2 + 1.0
        except ValueError:
            pass
    return BUILDING_HEIGHT.get(tags.get("building"), 6.0)


def build_bleachers(ring, materials, collection):
    """Aluminium bleachers on a concrete pad, rows rising away from the pitch.

    The mapped footprint is long along the touchline; the rows face the pitch
    (toward x = 0)."""
    xs = [p[0] for p in ring]
    ys = [p[1] for p in ring]
    near, far = (min(xs), max(xs)) if min(xs) > 0 else (max(xs), min(xs))
    y0, y1 = min(ys), max(ys)
    prism("solid:bleachers pad", [(min(xs), y0), (max(xs), y0), (max(xs), y1), (min(xs), y1)],
          -0.05, 0.15, materials["concrete"], collection)
    depth = abs(far - near)
    rows = 12
    seating_depth = min(depth, 10.0)
    step = seating_depth / rows
    direction = 1 if far > near else -1
    inset = 3.0
    for row in range(rows):
        x0 = near + direction * (inset + row * step)
        x1 = x0 + direction * step
        top = 0.45 + row * 0.42
        bench = [(min(x0, x1), y0 + 2), (max(x0, x1), y0 + 2), (max(x0, x1), y1 - 2), (min(x0, x1), y1 - 2)]
        prism(f"solid:bleachers row {row + 1}", bench, top - 0.06, top, materials["aluminium"], collection)
        riser = [(x0, y0 + 2), (x0 + direction * 0.04, y0 + 2), (x0 + direction * 0.04, y1 - 2), (x0, y1 - 2)]
        prism(f"bleachers riser {row + 1}", clean_ring(riser), 0.15, top - 0.06, materials["aluminium"], collection)
    # Back rail and supports.
    back = near + direction * (inset + seating_depth)
    rail_top = 0.45 + rows * 0.42 + 1.0
    for y in np.linspace(y0 + 2, y1 - 2, 12):
        prism("solid:bleachers post", [(back - 0.05, y - 0.05), (back + 0.05, y - 0.05), (back + 0.05, y + 0.05), (back - 0.05, y + 0.05)],
              0.15, rail_top, materials["steel"], collection, cap=True)
    prism("bleachers rail", [(back - 0.03, y0 + 2), (back + 0.03, y0 + 2), (back + 0.03, y1 - 2), (back - 0.03, y1 - 2)],
          rail_top - 0.05, rail_top, materials["steel"], collection)
    # Press box at the middle of the back row.
    mid = (y0 + y1) / 2
    box = [(back, mid - 6), (back + direction * 4, mid - 6), (back + direction * 4, mid + 6), (back, mid + 6)]
    prism("solid:press box", clean_ring(box), rail_top - 1.0, rail_top + 2.6, materials["siding"], collection)
    prism("press box roof", clean_ring([(back - direction * 0.6, mid - 6.5), (back + direction * 4.4, mid - 6.5),
                                        (back + direction * 4.4, mid + 6.5), (back - direction * 0.6, mid + 6.5)]),
          rail_top + 2.6, rail_top + 2.8, materials["roof"], collection)
    window = [(back - direction * 0.02, mid - 5), (back, mid - 5), (back, mid + 5), (back - direction * 0.02, mid + 5)]
    prism("press box window", clean_ring(window), rail_top - 0.2, rail_top + 1.8, materials["glass"], collection)


# Around Field #3's turf (73.5 x 123.4 m mapped).
FENCE_HALF_X = 40.0
FENCE_HALF_Y = 65.0
# NEXT 2026 Rules 2.2: each alliance's Base is a canopy tent about
# 20 ft wide x 10 ft deep. Red at the south end, Blue at the north.
BASE_WIDTH = 6.1
BASE_DEPTH = 3.05
BASE_CENTRES = {"red": (-18.0, -(PITCH_HALF_LENGTH + 6.0)), "blue": (-18.0, PITCH_HALF_LENGTH + 6.0)}


def box(name, x0, y0, x1, y1, z0, z1, mat, collection):
    return prism(name, [(x0, y0), (x1, y0), (x1, y1), (x0, y1)], z0, z1, mat, collection)


def build_field_furniture(materials, collection):
    """Competition-day field as in the NEXT2026 footage: a 3 m black vinyl
    chain-link fence around the turf (gate on the west side), four light
    towers, a scoreboard past the north end, pop-up canopies and a bench on
    the west apron. Positions are representative, not surveyed."""
    post = materials["post_black"]
    corners = [(-FENCE_HALF_X, -FENCE_HALF_Y), (FENCE_HALF_X, -FENCE_HALF_Y),
               (FENCE_HALF_X, FENCE_HALF_Y), (-FENCE_HALF_X, FENCE_HALF_Y)]
    runs = []
    for a, b in zip(corners, corners[1:] + corners[:1]):
        if a[0] == b[0] == -FENCE_HALF_X:
            # Gate: 4 m gap at midfield on the west side.
            runs += [(a, (a[0], 2.0)), ((a[0], -2.0), b)] if a[1] > b[1] else [(a, (a[0], -2.0)), ((a[0], 2.0), b)]
        else:
            runs.append((a, b))
    for a, b in runs:
        length = math.dist(a, b)
        d = (np.array(b) - np.array(a)) / max(length, 1e-6)
        n = np.array([-d[1], d[0]]) * 0.01
        for t in np.linspace(0, length, max(2, int(length / 3.0) + 1)):
            px, py = np.array(a) + d * t
            box("solid:fence post", px - 0.04, py - 0.04, px + 0.04, py + 0.04, 0.0, 3.05, post, collection)
        quad = clean_ring([tuple(np.array(a) + n), tuple(np.array(b) + n), tuple(np.array(b) - n), tuple(np.array(a) - n)])
        prism("solid:perimeter fence", quad, 0.03, 3.0, materials["fence_black"], collection)
        rail = clean_ring([tuple(np.array(a) + n * 3), tuple(np.array(b) + n * 3), tuple(np.array(b) - n * 3), tuple(np.array(a) - n * 3)])
        prism("fence top rail", rail, 2.97, 3.05, post, collection)

    for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        x, y = sx * (FENCE_HALF_X + 4.0), sy * 42.0
        box("solid:light tower", x - 0.2, y - 0.2, x + 0.2, y + 0.2, 0.0, 21.0, materials["steel"], collection)
        # Lamp array tilted toward the field: three rows of four heads.
        for row in range(3):
            for col in range(4):
                hx = x - sx * 0.6 + (col - 1.5) * 0.0
                hy = y + (col - 1.5) * 0.7
                hz = 21.0 + row * 0.75
                box("light head", hx - 0.3, hy - 0.28, hx + 0.3, hy + 0.28, hz, hz + 0.5, materials["lamp"], collection)
        box("light crossarm", x - 0.1, y - 1.5, x + 0.1, y + 1.5, 20.8, 23.4, materials["steel"], collection)

    sy = FENCE_HALF_Y + 9.0
    for px in (-2.2, 2.2):
        box("solid:scoreboard post", px - 0.12, sy - 0.12, px + 0.12, sy + 0.12, 0.0, 4.5, materials["steel"], collection)
    box("solid:scoreboard", -3.6, sy - 0.25, 3.6, sy + 0.25, 4.5, 7.6, materials["scoreboard"], collection)
    box("scoreboard trim", -3.7, sy - 0.27, 3.7, sy - 0.24, 7.45, 7.75, materials["label_white"], collection)

    for alliance, (cx, cy) in BASE_CENTRES.items():
        build_base(alliance, cx, cy, materials, collection)
    box("solid:bench seat", -38.6, -26.0, -38.2, -22.0, 0.42, 0.47, materials["aluminium"], collection)
    for by in (-25.7, -22.3):
        box("solid:bench leg", -38.5, by - 0.04, -38.3, by + 0.04, 0.0, 0.42, materials["steel"], collection)


def build_base(alliance, cx, cy, materials, collection):
    """An alliance Base: a 20 x 10 ft pop-up canopy, open on the field side,
    with a folding table and the alliance banner along the valance."""
    hw, hd, eave = BASE_WIDTH / 2, BASE_DEPTH / 2, 2.2
    for lx in (cx - hw, cx, cx + hw):
        for ly in (cy - hd, cy + hd):
            box("solid:base leg", lx - 0.03, ly - 0.03, lx + 0.03, ly + 0.03, 0.0, eave, materials["steel"], collection)
    bm = bmesh.new()
    rim = [bm.verts.new(v) for v in ((cx - hw, cy - hd, eave), (cx + hw, cy - hd, eave),
                                      (cx + hw, cy + hd, eave), (cx - hw, cy + hd, eave))]
    ridge = [bm.verts.new((cx - hw + 0.8, cy, eave + 0.8)), bm.verts.new((cx + hw - 0.8, cy, eave + 0.8))]
    bm.faces.new((rim[0], rim[1], ridge[1], ridge[0]))
    bm.faces.new((rim[2], rim[3], ridge[0], ridge[1]))
    bm.faces.new((rim[1], rim[2], ridge[1]))
    bm.faces.new((rim[3], rim[0], ridge[0]))
    mesh = bpy.data.meshes.new(f"{alliance} base roof")
    bm.to_mesh(mesh)
    bm.free()
    new_object(f"{alliance} base roof", mesh, materials["canopy_white"], collection)
    ring = clean_ring([(cx - hw, cy - hd), (cx + hw, cy - hd), (cx + hw, cy + hd), (cx - hw, cy + hd)])
    prism(f"{alliance} base banner", ring, eave - 0.3, eave, materials[f"alliance_{alliance}"], collection, cap=False)
    # Back wall away from the field, so operators are shielded (rules 2.2).
    back = cy - hd if cy < 0 else cy + hd
    wall = clean_ring([(cx - hw, back - 0.01), (cx + hw, back - 0.01), (cx + hw, back + 0.01), (cx - hw, back + 0.01)])
    prism(f"solid:{alliance} base wall", wall, 0.0, eave - 0.3, materials["canopy_grey"], collection)
    table_y = back + (0.5 if cy < 0 else -0.5)
    box(f"solid:{alliance} base table", cx - 1.2, table_y - 0.38, cx + 1.2, table_y + 0.38, 0.72, 0.76,
        materials["siding"], collection)
    for tx in (cx - 1.1, cx + 1.1):
        box("base table leg", tx - 0.02, table_y - 0.3, tx + 0.02, table_y + 0.3, 0.0, 0.72, materials["steel"], collection)


class TreeKit:
    """Shared trunk and crown meshes, so the glTF instances them."""

    def __init__(self, materials):
        bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=1.0, depth=1.0, location=(0, 0, 0.5))
        self.trunk = bpy.context.active_object.data
        self.trunk.materials.append(materials["bark"])
        bpy.data.objects.remove(bpy.context.active_object)
        self.crowns = []
        for leaves in materials["leaves"]:
            bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=2, radius=1.0)
            mesh = bpy.context.active_object.data
            mesh.materials.append(leaves)
            bpy.data.objects.remove(bpy.context.active_object)
            self.crowns.append(mesh)

    def place(self, x, y, rng, collection, mapped=False):
        """Deciduous tree: a trunk and two or three overlapping crowns."""
        height = rng.uniform(8, 14) if mapped else rng.uniform(9, 18)
        trunk_h = height * 0.4
        trunk_r = 0.12 + height * 0.012
        trunk = bpy.data.objects.new("solid:tree trunk", self.trunk)
        trunk.location = (x, y, 0.0)
        trunk.scale = (trunk_r, trunk_r, trunk_h)
        collection.objects.link(trunk)
        crown = self.crowns[rng.randrange(len(self.crowns))]
        for k in range(rng.randint(2, 3)):
            r = height * rng.uniform(0.22, 0.32)
            leaves = bpy.data.objects.new("tree crown", crown)
            leaves.location = (x + rng.uniform(-0.25, 0.25) * r, y + rng.uniform(-0.25, 0.25) * r,
                               trunk_h + r * rng.uniform(0.5, 1.0) + k * r * 0.3)
            leaves.scale = (r, r, r * rng.uniform(0.75, 0.95))
            collection.objects.link(leaves)


def home_plate(ring):
    """The diamond's sharpest corner near 90 degrees, and the bearing into the field."""
    best = None
    n = len(ring)
    for i in range(n):
        p = np.array(ring[i])
        a = np.array(ring[i - 1]) - p
        b = np.array(ring[(i + 1) % n]) - p
        if np.linalg.norm(a) < 5 or np.linalg.norm(b) < 5:
            continue
        angle = math.degrees(math.acos(np.clip(a @ b / (np.linalg.norm(a) * np.linalg.norm(b)), -1, 1)))
        if best is None or abs(angle - 90) < best[0]:
            bisector = a / np.linalg.norm(a) + b / np.linalg.norm(b)
            best = (abs(angle - 90), tuple(p), math.atan2(bisector[1], bisector[0]))
    return best[1], best[2]


def move_to(obj, collection):
    for c in list(obj.users_collection):
        c.objects.unlink(obj)
    collection.objects.link(obj)


def build_surroundings(out, materials, features, field_way):
    collection = bpy.data.collections.new("mason_surroundings")
    bpy.context.scene.collection.children.link(collection)
    rng = random.Random(233194670)

    # Ground: natural grass to the site edge, sitting just below the sim's own
    # ground so the authored pitch and apron draw on top.
    edge = SITE_RADIUS + 40
    flat_patch("ground", [(-edge, -edge), (edge, -edge), (edge, edge), (-edge, edge)], -0.04,
               materials["grass"], collection, uv_scale=16.0)

    trees = []
    woods = []
    kit = TreeKit(materials)
    build_field_furniture(materials, collection)
    for feature in features:
        tags = feature["tags"]
        if feature.get("osm") == field_way:
            continue  # The NEXT pitch itself is authored in Rust.
        pts = []
        if "points" in feature:
            pts = clip_to_site(feature["points"])
            if len(pts) < 2:
                continue
            centre = np.mean(pts, axis=0)
            if math.hypot(*centre) > SITE_RADIUS:
                continue
        if "point" in feature:
            x, y = feature["point"]
            if math.hypot(x, y) > SITE_RADIUS:
                continue
            if tags.get("natural") == "tree":
                trees.append((x, y, True))
            elif tags.get("highway") == "street_lamp":
                prism("solid:street lamp", [(x - 0.08, y - 0.08), (x + 0.08, y - 0.08), (x + 0.08, y + 0.08), (x - 0.08, y + 0.08)],
                      0.0, 7.5, materials["steel"], collection)
                prism("street lamp head", [(x - 0.3, y - 0.15), (x + 0.3, y - 0.15), (x + 0.3, y + 0.15), (x - 0.3, y + 0.15)],
                      7.5, 7.7, materials["steel"], collection)
            elif tags.get("power") == "pole":
                prism("solid:utility pole", [(x - 0.15, y - 0.15), (x + 0.15, y - 0.15), (x + 0.15, y + 0.15), (x - 0.15, y + 0.15)],
                      0.0, 10.5, materials["bark"], collection)
            continue
        closed = feature.get("closed") and len(pts) >= 4
        ring = clean_ring(pts) if closed else None
        if "building" in tags and closed:
            height = building_height(tags)
            wall = materials["brick"] if tags["building"] in ("university", "apartments") else materials["siding"]
            prism(f"solid:building {tags.get('name', tags['building'])}", ring, 0.0, height, wall, collection, uv_scale=6.0)
            prism("roof", ring, height, height + 0.25, materials["roof"], collection)
        elif tags.get("leisure") == "bleachers" and closed:
            build_bleachers(ring, materials, collection)
        elif tags.get("leisure") == "pitch" and closed:
            if tags.get("sport") == "baseball":
                flat_patch("baseball outfield", ring, -0.02, materials["turf"], collection, uv_scale=4.0)
                (cx, cy), bearing = home_plate(ring)
                infield = [(cx + 29 * math.cos(bearing + a), cy + 29 * math.sin(bearing + a))
                           for a in np.linspace(-math.pi / 4, math.pi / 4, 12)]
                flat_patch("baseball infield", [(cx, cy)] + infield, -0.015, materials["clay"], collection, uv_scale=3.0)
            else:
                flat_patch(f"pitch {tags.get('sport', '')}", ring, -0.02, materials["turf"], collection, uv_scale=4.0)
        elif tags.get("amenity") == "parking" and closed:
            flat_patch("parking lot", ring, -0.025, materials["asphalt"], collection, uv_scale=5.0)
        elif tags.get("amenity") == "parking_space" and closed:
            # Paint the stall outline.
            for a, b in zip(ring, ring[1:] + ring[:1]):
                strip("stall line", [a, b], 0.1, -0.018, materials["paint"], collection)
        elif (tags.get("natural") == "wood" or tags.get("landuse") == "forest") and closed:
            woods.append(ring)
        elif tags.get("barrier") == "fence":
            for a, b in zip(pts, pts[1:]):
                length = math.dist(a, b)
                for t in np.linspace(0, 1, max(2, int(length / 3) + 1)):
                    px, py = a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t
                    prism("solid:fence post", [(px - 0.03, py - 0.03), (px + 0.03, py - 0.03), (px + 0.03, py + 0.03), (px - 0.03, py + 0.03)],
                          0.0, 1.8, materials["steel"], collection)
                d = np.array(b) - np.array(a)
                n = np.array([-d[1], d[0]]) / max(np.linalg.norm(d), 1e-6) * 0.005
                mesh_ring = clean_ring([tuple(np.array(a) + n), tuple(np.array(b) + n), tuple(np.array(b) - n), tuple(np.array(a) - n)])
                prism("solid:fence mesh", mesh_ring, 0.05, 1.8, materials["chain_link"], collection)
        elif "highway" in tags:
            kind = tags["highway"]
            width = ROAD_WIDTH.get(kind, 0.0)
            if width <= 0 or closed and kind not in ("footway", "service"):
                continue
            mat = materials["concrete"] if kind in ("footway", "cycleway", "path", "pedestrian") else materials["asphalt"]
            strip(f"road {kind}", pts, width, -0.022 if mat is materials["asphalt"] else -0.02, mat, collection,
                  uv_scale=4.0)

    # Scatter woods, never inside the sim's ground square or on mapped surfaces.
    for ring in woods:
        xs = [p[0] for p in ring]
        ys = [p[1] for p in ring]
        for _ in range(4000):
            x = rng.uniform(min(xs), max(xs))
            y = rng.uniform(min(ys), max(ys))
            if math.hypot(x, y) > SITE_RADIUS or max(abs(x), abs(y)) < WORLD_HALF + 8:
                continue
            if point_in_ring(x, y, ring):
                trees.append((x, y, False))
            if len(trees) > 900:
                break
    # Thin to roughly one crown per 7 m.
    kept = []
    for x, y, mapped in trees:
        if mapped or all(math.hypot(x - a, y - b) > 7.0 for a, b, _ in kept[-400:]):
            kept.append((x, y, mapped))
    for x, y, mapped in kept:
        kit.place(x, y, rng, collection, mapped)
    print(f"surroundings: {len(collection.objects)} objects, {len(kept)} trees")
    export(collection, out / "models" / "mason_surroundings.glb")
    return collection


# ---------------------------------------------------------------- export and preview


def export(collection, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    for obj in collection.all_objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = next(iter(collection.all_objects))
    bpy.ops.export_scene.gltf(
        filepath=str(path),
        export_format="GLB",
        use_selection=True,
        export_apply=True,
        export_yup=True,
        export_image_format="AUTO",
        export_extras=False,
    )
    print(f"wrote {path} ({path.stat().st_size / 1024:.0f} KiB)")


def render_previews(directory, bucket, surroundings, ball_image):
    directory.mkdir(parents=True, exist_ok=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    # Standard keeps the authored colours (#F96302, optic yellow) unshifted.
    scene.view_settings.view_transform = "Standard"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 48
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 720
    world = bpy.data.worlds.new("sky")
    world.use_nodes = True
    sky = world.node_tree.nodes.new("ShaderNodeTexSky")
    world.node_tree.links.new(sky.outputs["Color"], world.node_tree.nodes["Background"].inputs["Color"])
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.4
    scene.world = world
    sun_data = bpy.data.lights.new("sun", "SUN")
    sun_data.energy = 4.0
    sun = bpy.data.objects.new("sun", sun_data)
    sun.rotation_euler = (math.radians(50), 0, math.radians(30))
    scene.collection.objects.link(sun)

    # Placeholder pitch so the overview reads like the sim.
    pitch_col = bpy.data.collections.new("preview pitch")
    scene.collection.children.link(pitch_col)
    flat_patch("preview pitch", [(-WORLD_HALF, -WORLD_HALF), (WORLD_HALF, -WORLD_HALF), (WORLD_HALF, WORLD_HALF), (-WORLD_HALF, WORLD_HALF)],
               0.0, bpy.data.materials["turf"], pitch_col, uv_scale=4.0)

    def shoot(name, location, target, lens=35):
        cam_data = bpy.data.cameras.new(name)
        cam_data.lens = lens
        cam = bpy.data.objects.new(name, cam_data)
        cam.location = location
        direction = np.array(target) - np.array(location)
        cam.rotation_euler = (math.atan2(math.hypot(direction[0], direction[1]), -direction[2]), 0,
                              math.atan2(direction[1], direction[0]) - math.pi / 2)
        scene.collection.objects.link(cam)
        scene.camera = cam
        scene.render.filepath = str(directory / f"{name}.png")
        bpy.ops.render.render(write_still=True)

    bucket_view = bucket.objects
    for obj in bucket_view:
        obj.location.x += 0.0
    shoot("overview", (-120, -170, 95), (20, 0, 0), lens=28)
    shoot("east_bleachers", (10, -20, 6), (70, 0, 3), lens=35)

    # Close-up of a bucket and a ball on turf.
    for obj in surroundings.all_objects:
        obj.hide_render = True
    bpy.ops.mesh.primitive_uv_sphere_add(segments=64, ring_count=32, radius=0.0335, location=(0.35, 0.0, 0.0335))
    ball = bpy.context.active_object
    ball.data.materials.append(material("ball", image=ball_image, roughness=0.95))
    shoot("bucket_and_ball", (0.9, -0.9, 0.55), (0.12, 0.0, 0.17), lens=50)


def main():
    args = parse_args()
    out = args.out.resolve()
    reset_scene()
    textures = out / "textures"
    turf = turf_texture(textures / "turf.png")
    ball = tennis_ball_texture(textures / "tennis_ball.png")
    # Surround-only textures are embedded in the glb, not written next to it.
    scratch = Path(bpy.app.tempdir or "/tmp") / "zorvane_next_textures"
    materials = {
        "orange": material("homer_orange", HOMER_ORANGE, roughness=0.42),
        "blue": material("bucket_blue", BUCKET_BLUE, roughness=0.42),
        "label_white": material("label_white", "#F7F7F2", roughness=0.5),
        "fence_black": material("vinyl_chain_link", "#1C1F1C", roughness=0.6, alpha=0.5),
        "post_black": material("vinyl_post", "#202320", roughness=0.5),
        "canopy_white": material("canopy_white", "#EEEEEA", roughness=0.8),
        "canopy_grey": material("canopy_grey", "#3E4246", roughness=0.8),
        # Alliance banners. Red alliance red, Blue alliance navy, kept clear
        # of the bucket blue so detectors do not learn the tents as buckets.
        "alliance_red": material("alliance_red", "#B3202A", roughness=0.7),
        "alliance_blue": material("alliance_navy", "#1F2A44", roughness=0.7),
        "scoreboard": material("scoreboard", "#14171A", roughness=0.6),
        "lamp": material("lamp", "#E8EEF2", roughness=0.3),
        # Metals are authored as light dielectrics: the sim has no environment
        # map, so true metals would render near black.
        "steel": material("galvanized_steel", "#A9ADB1", roughness=0.45),
        "black_plastic": material("black_plastic", "#1C1C1C", roughness=0.55),
        "turf": material("turf", image=turf, roughness=0.95),
        "grass": material("grass", image=grass_texture(scratch / "grass.png"), roughness=0.95),
        "asphalt": material("asphalt", image=speckle_texture(scratch / "asphalt.png", "#3D3F42", 0.25, 21, "asphalt"), roughness=0.9),
        "concrete": material("concrete", image=speckle_texture(scratch / "concrete.png", "#B9B6AE", 0.12, 22, "concrete"), roughness=0.85),
        "clay": material("infield_clay", image=speckle_texture(scratch / "clay.png", "#A5643D", 0.15, 23, "clay"), roughness=0.95),
        "brick": material("brick", image=brick_texture(scratch / "brick.png"), roughness=0.85),
        "siding": material("siding", "#D9D3C4", roughness=0.7),
        "roof": material("roof", "#4A4C4F", roughness=0.8),
        "aluminium": material("aluminium", "#C4C8CC", roughness=0.4),
        "glass": material("glass", "#30404A", roughness=0.1),
        "paint": material("line_paint", "#F2F2EE", roughness=0.7),
        "chain_link": material("chain_link", "#8C9196", roughness=0.5, alpha=0.35),
        "bark": material("bark", "#4B3A2C", roughness=0.95),
        "leaves": [material(f"leaves {i}", c, roughness=0.9) for i, c in enumerate(("#3E6B2E", "#4B7A33", "#355E2A", "#5A7F38"))],
    }
    osm = json.loads((HERE / "mason_osm.json").read_text())
    bucket = build_bucket(out, materials, materials["orange"], "homer_bucket.glb", label=True)
    blue = build_bucket(out, materials, materials["blue"], "blue_bucket.glb", label=False)
    for obj in blue.all_objects:
        obj.location.x += 0.45
    surroundings = build_surroundings(out, materials, osm["features"], osm["field"])
    if args.preview:
        render_previews(args.preview.resolve(), bucket, surroundings, ball)


main()
