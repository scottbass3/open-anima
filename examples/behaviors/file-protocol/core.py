# SPDX-License-Identifier: AGPL-3.0-or-later
"""File protocol example.

The program writes an action to temp/actions.mddo:

    [show (1/0), 'path of the animation', 'P-(x,y)']

where P is 1 when (x, y) is the position of the center of the animation, 0 to keep the previous
position. It then writes 1 to activador.act; Open Anima executes the action and writes 0 back.
"""

import random
import sys
import time


def screen_size():
    if sys.platform == "win32":
        import ctypes

        user32 = ctypes.windll.user32
        user32.SetProcessDPIAware()
        return user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    return 1920, 1080


def act(show, x=0, y=0, positioned=True):
    with open("temp/actions.mddo", "w") as f:
        f.write(f"[{int(show)},'temp\\ball.png','{int(positioned)}-({x},{y})']")
    with open("activador.act", "w") as f:
        f.write("1")


def main():
    width, height = screen_size()
    half = 48
    while True:
        edge = random.choice(["top", "bottom", "left", "right"])
        if edge in ("top", "bottom"):
            x = random.randint(half, width - half)
            y = half if edge == "top" else height - half
        else:
            x = half if edge == "left" else width - half
            y = random.randint(half, height - half)
        print(f"{edge}: {x},{y}", flush=True)
        act(True, x, y)
        time.sleep(1.5)
        act(False)
        time.sleep(random.uniform(1, 3))


if __name__ == "__main__":
    main()
