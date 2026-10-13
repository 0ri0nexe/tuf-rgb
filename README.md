# tuf-rgb

Open-source keyboard lighting for ASUS TUF laptops on Linux, built from a reverse engineering of Armoury Crate (see [docs/REVERSING.md](docs/REVERSING.md)).

Tested on: TUF Gaming F15 FX507VV (single-zone RGB, `asus-wmi` `kbd_rgb_mode`).
Should work on any TUF laptop where `/sys/class/leds/asus::kbd_backlight/kbd_rgb_mode` exists.

The reverse engineering of the lighting was achieved using [REA](https://github.com/morluto/rea) to test its abilities. Spoiler it's very powerful and proprietary binaries are basically done. You can find the full process of reversing in `docs/REVERSING.md`. The main thing to remember is that Armoury Crate was reversed to understand how it manages the lighting. 
Knowledge gained with this project will likely contribute to [asusctl](https://github.com/OpenGamingCollective/asusctl), when you're reading this you can probably use it as well.

> [!WARNING]  
> This software is touching to some critical parts of the laptop. Issue that could harm hardware or firmware are unlikely but not strictly impossible and this software have no claim to be perfectly safe. Therefor it will not take any resposability for any broken device.

## Features

* The 4 firmware modes (static, breathing, color cycle, strobe) with speed, optionally saved in the EC so they survive reboot, plus raw access to the other mode bytes.
* Host-rendered effects: breathe (multi-color), rainbow, gradient, cycle, strobe, heartbeat, candle, police, storm, CPU load, temperature, battery.
* Notifications: blink a color over the running effect (`tuf-rgb notify`), handy in scripts.
* Power states: lit at boot / awake / sleep / keyboard.
* Profiles in `~/.config/tuf-rgb/config.toml`, gamma, software intensity, frame rate.
* `tuf-rgbd` daemon (runs as your user), `tuf-rgb` CLI, `tuf-rgb-gui` editor with live preview.

## Install

```
./install.sh
```

Builds in release mode, installs to `~/.local/bin`, installs a udev rule giving the `tufrgb` group write access to the LED attributes, and enables the `tuf-rgbd` user service. Log out and back in once for the group to apply.

## Usage

**Cli :**

```
tuf-rgb status
tuf-rgb color '#ff3c00'               # temporary
tuf-rgb color cyan --save               # stored in the EC
tuf-rgb hw breathing -c red -s slow
tuf-rgb effects                         # list effects and parameters
tuf-rgb effect breathe colors=#ff0000,#0000ff period=3 min=0.1
tuf-rgb effect temperature cold=#0040ff hot=red min_c=45 max_c=95
tuf-rgb profile save night breathe colors=#300060 period=8
tuf-rgb profile apply night
tuf-rgb notify green --count 3
tuf-rgb power --sleep off --save
tuf-rgb settings --fps 30 --gamma 2.2 --intensity 0.8
tuf-rgb-gui
```

Without the daemon, `color`, `hw`, `power` and `brightness` write sysfs directly and `effect` renders in the foreground. `--dry-run` (or `TUF_RGB_DRY_RUN=1`) prints writes instead.

## Notes

* Software effects are one ACPI call into the EC per frame. 30 fps is plenty; temporary writes (cmd `0xB3`) never touch the EC's persistent storage. Only `--save` uses `0xB4`.
* The keyboard hotkeys still control hardware brightness (0..3) on top of everything.

## Contributing

This software is not actively maintained but feel free to contribute if you want, every input will be reviewed and merged if it is improving the software.