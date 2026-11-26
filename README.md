# logirate

Per-app polling rate (and DPI) for Logitech mice on Windows, without G HUB.

8000 Hz is great in a competitive shooter and can make other games stutter. logirate watches which app is in
focus (or running) and switches the mouse's report rate to match your rules, for example 8000 Hz in CS2 and the
mouse's own profile everywhere else.

## How it works

- Talks to the mouse directly over Logitech's HID++ 2.0 protocol through the LIGHTSPEED receiver (or a cable).
- Changing the rate puts the mouse into its temporary **host mode** (feature `0x8100`) and sets the rate with
  `0x8061` (extended report rate, up to 8 kHz) or `0x8060` (up to 1 kHz). The DPI from your onboard profile is
  carried over unless a rule sets its own.
- `onboard` hands control back to the mouse's saved profile.
- **Nothing is written to the mouse's flash.** Unplugging it or switching it off always brings back its own profile.
- If the mouse loses host mode (power cycle, sleep), logirate notices within a few seconds and re-applies.

## Parts

| File | What it is |
|---|---|
| `logirate-gui.exe` | Window: mouse status, rule editor, start/stop the service, start at login |
| `logirated.exe` | The background service (no window). Logs to `%LOCALAPPDATA%\logirate\logirate.log` |
| `logirate.exe` | Command line: `info`, `set`, `rules`, `add`, `remove`, `default`, `watch` |

## Configuration

`%APPDATA%\logirate\config.toml`, reloaded automatically on save:

```toml
default = "onboard"      # when no rule matches: "onboard" or a rate
poll_ms = 500

[[rule]]
name = "CS2"
process = "cs2.exe"      # one or a list; * and ? wildcards; a pattern with \ matches the full path
rate = 8000
# dpi = 800              # optional
# title = "counter"      # optional: window title must contain this
# when = "running"       # default "focused"
```

Rates: 125, 250, 500, 1000, 2000, 4000, 8000 (whatever your mouse supports, see `logirate info`), or `onboard`.

```powershell
logirate info
logirate add cs2.exe 8000
logirate add "D:\Games\*" 2000 --name "Everything in D:\Games"
logirate add obs64.exe 1000 --running
logirate rules
```

## Build

```powershell
cargo build --release
```

Needs Rust with the MSVC toolchain (Visual Studio Build Tools, C++).

## Notes

- Tested with a LIGHTSPEED receiver (`046d:c54d`). Other HID++ 2.0 mice with `0x8060`/`0x8061` should work.
- Don't run G HUB at the same time; both would fight over the mouse.
- In host mode the mouse's onboard button mappings and profile switching are not active.

## License

MIT
