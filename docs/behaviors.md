# Behaviors

A behavior is a program that drives animations on the desktop: a character walking along the
screen edges, a cat that sleeps at night, a reminder that pops up every hour… Open Anima starts
the program, reads its commands and displays the animations.

## Package layout

```
my-behavior/
├── manifest.toml
├── main.py            # or core.exe, index.js, …
└── assets/
    ├── walk.gif
    └── meow.ogg
```

`manifest.toml`:

```toml
name = "My behavior"
author = "You"
version = "1.0"
description = "What it does."
preview = "assets/walk.gif"   # shown in the Behaviors tab
command = ["python", "main.py"]
protocol = "stdio"            # "stdio", "files" or "auto" (both, the default)
```

`command[0]` is looked up in the package folder first, then in `PATH`. The program runs with the
package folder as its working directory. Paths in commands are relative to that folder.

To distribute a behavior, use **Create package…** in the Behaviors tab: it zips the folder into
a `.oapack` file (and writes a `manifest.toml` if there is none). **Load package…** installs a
`.oapack`, **Load folder…** uses a folder in place, which is handy while developing.

Open Anima asks for confirmation before running a behavior for the first time, and again when
its command changes. A behavior is a regular program running with your permissions: only run
behaviors whose source you trust. Behaviors are stopped when Open Anima exits, even if it
crashes.

## The stdio protocol

The program prints one JSON object per line on its standard output (don't forget to flush).
Lines that are not JSON objects, and everything printed on standard error, go to the log
shown in the Behaviors tab.

Coordinates are **physical screen pixels**, as returned by the operating system (call
`SetProcessDPIAware()` on Windows if you use `GetSystemMetrics`). Several animations can be
shown at once, each in its own named `slot` (`"main"` when omitted).

| Command | Fields | Effect |
| --- | --- | --- |
| `show` | `file`, `slot`, `x`, `y`, `anchor`, `scale`, `flip_h`, `click_through`, `draggable` | Shows `file` in `slot`, restarting it from its first frame. Without `x`/`y` the slot keeps its position (or is centered on the screen). |
| `move` | `slot`, `x`, `y`, `anchor` | Moves the slot. |
| `hide` | `slot` | Removes the slot. |
| `clear` | | Removes every slot. |
| `sound` | `file`, `volume` (0‥1) | Plays a sound once. |

- `anchor`: `"center"` (default) or `"topleft"`: which point of the animation `x`/`y` refers to.
- `scale`: size multiplier, default 1.
- `click_through`: mouse clicks go through the animation (default `true`).
- `draggable`: the user can move it (default `false`).

Example (Python):

```python
import json

def send(**command):
    print(json.dumps(command), flush=True)

send(op="show", slot="cat", file="assets/walk.gif", x=200, y=900)
send(op="move", slot="cat", x=260, y=900)
send(op="sound", file="assets/meow.ogg", volume=0.5)
send(op="hide", slot="cat")
```

See `examples/behaviors/bouncer`.

## The file protocol (compatibility)

Some desktop animation tools use two files to talk to their behavior programs. Open Anima
supports it so that existing packs work when their folder is loaded with **Load folder…** (a
folder without manifest is accepted if it contains an `.exe` at its root; it then uses this
protocol).

1. The program writes the action to `temp/actions.mddo`:

   ```
   [1,'temp\\walk.gif','1-(500,300)']
   ```

   - first field: `1` to show the animation, `0` to hide it;
   - second field: the animation, relative to the package folder;
   - third field: `1-(x,y)` to put the **center** of the animation at `(x, y)`, `0-(x,y)` to
     keep the current position.

2. It then writes `1` into `activador.act` (at the root of the package folder).

3. Open Anima reads the action, applies it and writes `0` back into `activador.act`.

There is a single slot with this protocol. See `examples/behaviors/file-protocol`.
