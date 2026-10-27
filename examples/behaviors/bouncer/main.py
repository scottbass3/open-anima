# SPDX-License-Identifier: AGPL-3.0-or-later
"""Bounces a ball around the screen using the Open Anima stdio protocol.

Every line printed on stdout that starts with `{` is a command; anything else ends up in the
behavior's log in Open Anima. Coordinates are physical screen pixels.
"""

import json
import random
import sys
import time

BALL = 96


def screen_size():
    if sys.platform == "win32":
        import ctypes

        user32 = ctypes.windll.user32
        user32.SetProcessDPIAware()  # report physical pixels on scaled displays
        return user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    return 1920, 1080


def send(**command):
    print(json.dumps(command), flush=True)


def main():
    width, height = screen_size()
    print(f"screen: {width}x{height}", flush=True)

    x, y = random.uniform(0, width - BALL), random.uniform(0, height - BALL)
    vx, vy = 420.0, 300.0
    send(op="show", slot="ball", file="assets/ball.png", x=x, y=y, anchor="topleft")

    last = time.monotonic()
    while True:
        time.sleep(1 / 60)
        now = time.monotonic()
        dt, last = now - last, now
        x, y = x + vx * dt, y + vy * dt
        if not 0 <= x <= width - BALL:
            vx = -vx
            x = min(max(x, 0), width - BALL)
        if not 0 <= y <= height - BALL:
            vy = -vy
            y = min(max(y, 0), height - BALL)
        send(op="move", slot="ball", x=round(x), y=round(y), anchor="topleft")


if __name__ == "__main__":
    main()
