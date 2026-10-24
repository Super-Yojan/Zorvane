"""Regenerate the NEXT intro voice-over from `assets/cutscene/next_intro.json`.

macOS only: speech comes from the system `say` voice named in the script, and
ffmpeg encodes it as Ogg Vorbis (the format Bevy already decodes). Each beat's
`seconds` is rewritten to its clip length plus a short pause, so the camera and
captions stay in step with the narration.

    python3 simulator/tools/cutscene/make_voiceover.py
"""

import json
import subprocess
import sys
import tempfile
from pathlib import Path

ASSETS = Path(__file__).resolve().parents[2] / "assets"
SCRIPT = ASSETS / "cutscene" / "next_intro.json"
PAUSE_SECONDS = 0.5


def duration(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
        check=True, capture_output=True, text=True,
    )
    return float(out.stdout.strip())


def main():
    script = json.loads(SCRIPT.read_text())
    with tempfile.TemporaryDirectory() as scratch:
        for index, beat in enumerate(script["beats"], start=1):
            spoken = Path(scratch) / f"beat{index}.aiff"
            subprocess.run(
                ["say", "-v", script["voice"], "-r", str(script["rate"]), "-o", str(spoken), beat["narration"]],
                check=True,
            )
            target = ASSETS / beat["audio"]
            target.parent.mkdir(parents=True, exist_ok=True)
            # FFmpeg's built-in Vorbis encoder needs stereo and -strict -2.
            subprocess.run(
                ["ffmpeg", "-v", "error", "-y", "-i", str(spoken), "-ac", "2", "-ar", "44100",
                 "-c:a", "vorbis", "-strict", "-2", "-q:a", "5", str(target)],
                check=True,
            )
            beat["seconds"] = round(duration(target) + PAUSE_SECONDS, 2)
            print(f"{beat['audio']}: {beat['seconds']} s")
    SCRIPT.write_text(json.dumps(script, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    sys.exit(main())
