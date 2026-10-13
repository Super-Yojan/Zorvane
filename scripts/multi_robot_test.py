#!/usr/bin/env python3
"""Start Zorvane with 4 default Terra robots."""

import os
from pathlib import Path
import subprocess
import sys


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    environment = os.environ.copy()

    # A roster overrides TERRA_ROVER_COUNT. Remove it so this launcher always
    # starts the full fleet using the default vehicle configuration.
    environment.pop("ZORVANE_ROBOT_CONFIG", None)
    environment["TERRA_ROVER_COUNT"] = "4"

    try:
        return subprocess.run(
            ["cargo", "run", "-p", "zorvane"],
            cwd=repo_root,
            env=environment,
            check=False,
        ).returncode
    except FileNotFoundError:
        print("Could not find cargo. Install Rust and ensure cargo is on PATH.", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
