"""Fold the repo's existing Markdown into the MkDocs nav at build time.

The sources stay where GitHub renders them (README, MIGRATION, simulator/*.md).
This hook copies them under docs/reference/ and rewrites sibling links so
`mkdocs build --strict` can check them. Edit the source files, not the copies.
"""

from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "docs" / "reference"

SOURCES = {
    "readme.md": ROOT / "README.md",
    "migration.md": ROOT / "MIGRATION.md",
    "vehicles.md": ROOT / "VEHICLES.md",
    "world.md": ROOT / "simulator" / "WORLD.md",
    "next.md": ROOT / "simulator" / "NEXT.md",
    "zenoh.md": ROOT / "simulator" / "ZENOH.md",
    "depth-camera.md": ROOT / "simulator" / "DEPTH_CAMERA.md",
}

# Longer patterns first so a prefix is not rewritten twice.
REPLACEMENTS = (
    ("](simulator/ZENOH.md)", "](zenoh.md)"),
    ("](simulator/NEXT.md)", "](next.md)"),
    ("](simulator/WORLD.md)", "](world.md)"),
    ("](../VEHICLES.md)", "](vehicles.md)"),
    ("](VEHICLES.md)", "](vehicles.md)"),
    ("](MIGRATION.md)", "](migration.md)"),
    ("](ZENOH.md", "](zenoh.md"),
    ("](NEXT.md)", "](next.md)"),
    ("](WORLD.md", "](world.md"),
    ("](docs/world-preview.png)", "](world-preview.png)"),
)


def on_pre_build(config):
    del config
    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    for name, src in SOURCES.items():
        text = src.read_text(encoding="utf-8")
        for old, new in REPLACEMENTS:
            text = text.replace(old, new)
        source = src.relative_to(ROOT).as_posix()
        banner = (
            f"<!-- Folded from {source} by mkdocs_hooks.py. Edit that file. -->\n\n"
        )
        (OUT / name).write_text(banner + text, encoding="utf-8")
    image = ROOT / "simulator" / "docs" / "world-preview.png"
    shutil.copyfile(image, OUT / "world-preview.png")
