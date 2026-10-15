"""Print one rover's pose until the process is stopped."""

import argparse
import time

from .client import connect


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rover", type=int, default=0)
    parser.add_argument("--seconds", type=float, default=None)
    args = parser.parse_args()
    client = connect()

    def print_pose(pose) -> None:
        print(f"{pose.x:.4f} {pose.y:.4f} {pose.heading:.4f}", flush=True)

    client.subscribe_pose(args.rover, print_pose)
    if args.seconds is None:
        try:
            while True:
                time.sleep(1)
        except KeyboardInterrupt:
            pass
    else:
        time.sleep(args.seconds)
    client.close()


if __name__ == "__main__":
    main()
