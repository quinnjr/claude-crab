# claude-crab

Clawd walks in a strip just above your KDE Plasma panel, animating to reflect
what Claude Code is doing.

Idle, it sleeps in a corner. When a session starts working it walks; the gait
changes with the tool in use. When Claude needs your input it stops, turns to
face you, and waves.

## What it is

A standalone Qt6/QML application, **not a plasmoid**. `layer-shell-qt` exposes no
QML import, so a C++ entry point is required to configure the layer surface.
Running outside `plasmashell` also means a crash here can't take your panel down.

The window is a transparent, click-through `wlr-layer-shell` surface anchored to
the bottom edge, with `exclusiveZone = 0` so the compositor places it directly
above the panel rather than behind it.

## Requirements

- KDE Plasma 6 on Wayland (there is an X11 fallback, but it is a degraded path)
- Qt 6.6+, `layer-shell-qt`
- Python 3 with Pillow, at build time only, to generate the sprite sheet

## Build

```sh
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=~/.local
cmake --build build
ctest --test-dir build --output-on-failure
cmake --install build
```

That installs `claude-crab`, `claude-crab-hooks`, the `.desktop` file, and a
systemd user unit. An Arch `PKGBUILD` for a system-wide install is in `packaging/`.

## Flatpak

```sh
flatpak install --user flathub org.kde.Platform//6.9 org.kde.Sdk//6.9
flatpak-builder --user --force-clean --install \
  build-flatpak packaging/dev.quinnjr.claude-crab.yml
flatpak run dev.quinnjr.claude-crab
```

Two things in the manifest are not obvious.

**The state directory.** Claude Code runs on the host and is not sandboxed, so
its hooks write to the host's `~/.local/state/claude-crab/inbox`. Inside the
sandbox `XDG_STATE_HOME` points at `~/.var/app/<id>/.local/state`, so honouring
it would leave the crab watching an empty directory with nothing to explain the
silence. `CrabConfig::inboxDir()` skips `XDG_STATE_HOME` when `/.flatpak-info`
exists, and the manifest binds the host directory through at the same path.
`$CLAUDE_CRAB_STATE_DIR` overrides both, which is what a host with a
non-default `XDG_STATE_HOME` needs.

**Pinned dependencies.** `layer-shell-qt` is not in `org.kde.Platform`, so the
manifest builds it — pinned to 6.5.5, because the 6.6 series onward needs Qt
6.10 while `org.kde.Sdk//6.9` carries Qt 6.9.3. Pillow is vendored as a wheel
purely to run `gen_sprites.py` at build time, and is cleaned out of the result.

`--socket=wayland` is granted but X11 is not: layer-shell is a Wayland protocol
with no X11 equivalent, so the fallback path is of no use inside a sandbox.

Still to do before a Flathub submission: swap the `dir` source for a tagged
`git` source, add screenshots to the metainfo, and decide whether the desktop
file should keep `NoDisplay=true` — Flathub expects a launchable entry, but a
menu item for a background service is its own kind of wrong.

The unit's `ExecStart` and install location are both derived from the prefix.
systemd searches a fixed set of directories for user units and
`<prefix>/lib/systemd/user` is not among them when the prefix is inside `$HOME`,
so a `$HOME` prefix installs to `<prefix>/share/systemd/user` instead. Override
with `-DSYSTEMD_USER_UNIT_DIR=...` if your layout differs.

## Run it as a service

```sh
systemctl --user enable --now claude-crab
systemctl --user status claude-crab
journalctl --user -u claude-crab -f
```

## Install the hooks

The crab learns what Claude Code is doing from hook events. `crab_hooks.py`
registers them:

```sh
python3 tools/crab_hooks.py --dry-run install   # show the diff first
python3 tools/crab_hooks.py install
python3 tools/crab_hooks.py status
python3 tools/crab_hooks.py uninstall
```

By default it patches **every** profile under
`$XDG_DATA_HOME/claude-profiles/`. That is deliberate: with more than one
profile and a switchable default, patching only the active one would leave the
other profile's sessions invisible to the crab. Override with `--config-dir`,
`--profile <name>`, `$CLAUDE_HOME`, or `$CLAUDE_CONFIG_DIR`.

Every command it writes ends in an inert `# claude-crab:v1` shell comment.
`uninstall` removes only entries carrying that marker, so a hand-written hook is
never touched, and a timestamped backup is taken before the first change to each
file.

## Run

```sh
./build/claude-crab                              # normal operation
./build/claude-crab --sprite fancy               # top hat and monocle
./build/claude-crab --demo                       # cycle every animation
./build/claude-crab --replay fixtures/session.jsonl
```

### Seeing the logs

Under the service, everything lands in the journal — `journalctl --user -u
claude-crab`. Qt does this whenever it detects a systemd session, which is also
why running the binary straight from a terminal looks silent. Force it to
stderr:

```sh
QT_FORCE_STDERR_LOGGING=1 QT_LOGGING_RULES='claude.crab=true' ./build/claude-crab
```

Without the `.desktop` file installed you will also see a harmless
`org.freedesktop.portal.Error.Failed` line about the app id; `cmake --install`
puts the file in place and it goes away.

## How the signal gets here

```
claude session ──hook──▶ inbox/<ns>.json ──watcher──▶ SessionTracker ──signal──▶ CrabBrain
```

The hook command is pure coreutils, so there is no interpreter or binary startup
cost on every tool call:

```sh
d="$HOME/.local/state/claude-crab/inbox"; mkdir -p "$d"; f="$d/$(date +%s%N)"
cat > "$f.tmp" && mv "$f.tmp" "$f.json"
```

It writes the raw payload and renames it into place atomically, so the watcher
never reads a partial file. One file per event rather than a shared append log:
`PreToolUse` payloads embed `tool_input`, which for an `Edit` routinely exceeds
`PIPE_BUF`, and appends past that size are not atomic — concurrent sessions
would corrupt each other's records.

`SessionTracker` folds every session into one aggregate state, priority
`WaitingInput` > `Working` > `Idle`, and retires any session silent for ten
minutes (a `SIGKILL`ed session never sends `Stop` or `SessionEnd`).

## Behaviour

| State | Animation |
| --- | --- |
| idle | walks to its corner, then `sleep` |
| working, `Bash` | `scuttle` |
| working, `Edit`/`Write` | `creep` |
| working, `Read`/`Grep`/`Glob` | `walk` |
| working, no tool | `think` — planted, eyes drifting upward |
| waiting for input | `wave`, facing you |
| session finished | `celebrate` once |
| tool failed | `tumble` once |

## Right-click menu

Right-clicking the character opens a menu for switching sprite variants. The
choice is written to the config file straight away, so it survives the restarts
a systemd-managed service makes routine.

The window is not globally click-through. Input is confined to the character's
own rectangle by a window mask that tracks it as it walks, so a right click on
the character reaches the menu while every other pixel of the strip still passes
clicks through to whatever is underneath. The mask is updated on a timer and
ignores sub-threshold moves, because each change is a Wayland commit.

The menu is drawn inside the window rather than as a popup: the window reserves
`menuHeadroom` pixels of transparent space above the walking band for it. A
popup would be a second surface parented to a layer-shell surface, which is far
more fragile for no benefit here.

## Keeping the inbox bounded

The hooks write whether or not the crab is running, so two limits apply.

**Per file.** `PreToolUse` payloads embed `tool_input`, which for a large `Write`
or `Edit` runs to megabytes. The hook caps each file at 16 KiB with
`truncate -s '<N'`. Claude Code emits `session_id`, `hook_event_name` and
`tool_name` before `tool_input`, so the cap keeps everything the crab needs;
when a file is cut mid-string, `SessionTracker` salvages those fields by scanning
rather than dropping the event. `tool_response` sits after `tool_input`, so a
truncated payload loses only the error blip.

`truncate` is used rather than `head -c`, which would close the pipe early and
hand the writing process an EPIPE for every oversized payload.

**Per directory.** On startup and once a minute the crab drops events older than
`inboxMaxAgeMinutes` — a session state from an hour ago says nothing about now —
and then drops the oldest until the directory is under `inboxMaxMegabytes`.
Abandoned `.tmp` files from a hook killed mid-write are swept the same way.
Anything dropped is logged; the crab never discards events silently.

Installer backups are bounded too: the newest five are kept per settings file.

## Configuration

`~/.config/claude-crab.json`, every key optional:

```json
{
  "stripHeight": 72,
  "crabScale": 1.0,
  "output": "DP-1",
  "sleepCorner": "right",
  "sprite": "default",
  "menuHeadroom": 220,
  "staleTimeoutMinutes": 10,
  "inboxMaxAgeMinutes": 60,
  "inboxMaxMegabytes": 32,
  "reactions": {
    "waiting": true,
    "finished": true,
    "error": true,
    "toolFlavour": true
  }
}
```

A missing or malformed file yields defaults rather than an error, and an
unrecognised value for a constrained key (`sleepCorner`, `sprite`) logs a warning
and falls back rather than failing to start.

### Sprite variants

`sprite` selects the look: `"default"`, or `"fancy"` for a top hat and monocle.
`--sprite <variant>` overrides the config file for one run, which is the quick
way to compare them. An unknown value on the command line is a hard error —
there it is a typo worth surfacing, whereas in a config file it should not stop
the crab from running.

Both sheets are compiled into the binary and share a single manifest: they
differ only in what is drawn inside a frame, never in the row and frame layout,
so switching costs nothing at runtime.

## The art

`tools/gen_sprites.py` generates `spritesheet.png` and `manifest.json` at build
time; the PNG is not checked in, so art and manifest cannot drift apart. The
manifest is the contract — a hand-drawn replacement sheet needs no QML change so
long as the row/frame layout matches.

The character is drawn as flat terracotta `#D06A4B` blocks on a coarse grid,
with no outline and no shading: a 12x8 cell body — torso, two square eyes, a nub
on each side, four stubby legs — on a 16x16 cell canvas at 4px per cell.

Poses are expressed in grid cells rather than pixels, because drawing off-grid
is what makes a blocky sprite look wrong. The few things that move by pixels
(hops, blinks, the lean on a fast gait) say so explicitly. Tumble rotation snaps
to quarter turns for the same reason — nearest-neighbour rotation at an
arbitrary angle shreds a blocky sprite into loose pixels.

## Licence

MIT.
