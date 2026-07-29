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

`packaging/flathub/dev.quinnjr.claude-crab.yml` is the submission manifest: it
is identical except that the `claude-crab` module builds from the tagged
release, pinned to both tag and commit, rather than from the working tree.

The desktop file keeps `NoDisplay=true`: this is a background companion with no
window, so a launcher entry would either do nothing visible or start a second
crab. Flathub's linter treats that as an error and grants exceptions for
background services, so the submission needs one requested.

`packaging/flathub/README.md` covers that and the remaining blockers — chiefly
metainfo screenshots — along with how to run both halves of the linter.

Note that the Flatpak asks for no access to Claude Code's `settings.json`. Hook
entries are arbitrary shell commands, so write access to that file amounts to
unsandboxed code execution on the host. Flatpak users install hooks host-side
with `claude-crab-hooks`; the crab itself only reads events.

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

It resolves the config directory in this order:

| | Target |
| --- | --- |
| `--config-dir <path>` | exactly that directory |
| `--profile <name>` | `$XDG_DATA_HOME/claude-profiles/<name>` |
| `--all` | every profile under `$XDG_DATA_HOME/claude-profiles/` |
| `$CLAUDE_HOME` | its value |
| `$CLAUDE_CONFIG_DIR` | its value |
| *(default)* | `~/.claude` |

`~/.claude` is the default because it is the one location every Claude Code
install has. Multi-profile setups opt in with `--all` or `--profile` rather than
being discovered behind your back — and if you use `claude-profiles`, its shell
wrapper exports `CLAUDE_CONFIG_DIR`, so a bare invocation already lands on the
active profile without any flag.

Every command it writes ends in an inert `# claude-crab:v1` shell comment.
`uninstall` removes only entries carrying that marker, so a hand-written hook is
never touched, and a timestamped backup is taken before the first change to each
file.

### From inside the Flatpak

The Flatpak holds no permission to Claude Code's config — see [Flatpak](#flatpak)
for why — so the hooks CLI needs it granted for the one invocation that installs
them. The crab itself never holds it.

```sh
# The default target, ~/.claude
flatpak run --unset-env=CLAUDE_CONFIG_DIR --unset-env=CLAUDE_HOME \
  --filesystem=~/.claude \
  --command=claude-crab-hooks dev.quinnjr.claude-crab install

# Or a claude-profiles setup, patching every profile
flatpak run --filesystem=xdg-data/claude-profiles \
  --command=claude-crab-hooks dev.quinnjr.claude-crab --all install
```

The `--unset-env` flags are not decoration. `flatpak run` passes your
environment through, so if the shell you launch from exports
`CLAUDE_CONFIG_DIR` — which the `claude-profiles` wrapper does — it outranks the
default and the command quietly targets that profile instead of `~/.claude`.
Drop the flags when that is what you want.

`--dry-run` works the same way and is worth running first. `status` and
`uninstall` need the same `--filesystem` grant, since they read and rewrite the
same file.

Grant only the path you actually use: `--filesystem=~/.claude` does not cover
`claude-profiles`, and vice versa. Get it wrong and the run says so —

```
/home/…/claude-profiles/personal: ERROR … is not an accessible directory
  - inside a Flatpak this usually means the path was not granted,
    e.g. --filesystem=/home/…/claude-profiles/personal
```

— rather than reporting every hook as missing, which is what an unreadable
directory would otherwise look like.

Discovery is sandbox-aware: inside a Flatpak `XDG_DATA_HOME` points at
`~/.var/app/<id>/data`, so `--all` and `--profile` deliberately ignore it and
read the host's `~/.local/share/claude-profiles` instead. Without that they
would resolve to an empty sandbox path and report phantom profiles.

Nothing stops you running the host copy instead — `tools/crab_hooks.py` is
stdlib-only Python and needs no install.

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

**Per directory, while the crab runs.** On startup and once a minute it drops
events older than `inboxMaxAgeMinutes` — a session state from an hour ago says
nothing about now — and then drops the oldest until the directory is under
`inboxMaxMegabytes`. Abandoned `.tmp` files from a hook killed mid-write are
swept the same way. Anything dropped is logged; the crab never discards events
silently.

**Per directory, while it does not.** The hooks fire whether or not the crab is
running, so the crab cannot be the only thing that cleans up: stop the service,
or uninstall it without removing the hooks, and the inbox would grow forever.
The hook therefore carries its own sweep, sampled on the last digit of the
timestamp so it runs about one write in ten and its cost stays out of the hot
path. It is a backstop pinned to the default age limit, not the policy —
`inboxMaxAgeMinutes` governs the crab, and the two only differ if you change it.

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

`sprite` selects the look: `"default"`, `"fancy"` for a top hat and monocle, or
`"party"` for a birthday hat.
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

Three variants are emitted — plain, `fancy` (top hat and monocle) and `party`
(birthday hat) — sharing one manifest, since they differ only in what is drawn
inside a frame.

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
