# claude-crab

Clawd walks along the bottom of your screen, over the panel, animating to reflect what Claude
Code is doing. Linux, macOS and Windows.

Idle, it sleeps in a corner. When a session starts working it walks; the gait
changes with the tool in use. When Claude needs your input it stops, turns to
face you, and waves.

## What it is

A standalone Rust binary, **not a plasmoid**. Drawing is done with
[`skia-rs`](https://crates.io/crates/skia-rs), a pure-Rust reimplementation of
Skia, so nothing here links Qt, KDE or any C++ toolkit. Running as its own
process also means a crash can't take your panel down.

There are two window backends, chosen at startup:

| Backend | Where | How it looks |
| --- | --- | --- |
| `layer-shell` | Sway, Hyprland, KWin, Wayfire, River | A transparent full-width `wlr-layer-shell` surface anchored to the bottom edge, `exclusiveZone = -1` so it hugs the true bottom of the screen and the crab walks over the panel. Input is confined to the character, so the rest of the strip stays click-through. |
| `floating` | macOS, Windows, X11, GNOME | A small always-on-top window that *is* the crab and moves with it. Nothing outside the character is covered, so no click-through trickery is needed. |

Set `CLAUDE_CRAB_BACKEND=floating` (or `layer-shell`) to override the choice.

Two caveats on the floating backend. Wayland gives clients no way to position
their own windows, so under GNOME the crab animates in place rather than walking
— it says so in the log. And neither Windows nor macOS exposes a work area to
`winit`, so if the taskbar or Dock covers the crab, lift it with `bottomMargin`
in the config.

## Requirements

- Linux (Wayland or X11), macOS, or Windows
- A GPU with Vulkan, Metal or DX12 — only the `floating` backend needs this
- Python 3 with Pillow, at build time only, to generate the sprite sheet

## Build

```sh
cargo build --release
cargo test
```

Then install `target/release/claude-crab` and `tools/crab_hooks.py` (as
`claude-crab-hooks`) somewhere on `PATH`. An Arch `PKGBUILD` that also installs
the `.desktop` file, icons and a systemd user unit is in `packaging/`.

## Packages

Every release carries prebuilt packages, built by `.github/workflows/packages.yml`:

- **Debian/Ubuntu**: `claude-crab_<version>_amd64.deb` — `apt install ./claude-crab_*.deb`
- **Fedora/openSUSE**: `claude-crab-<version>.x86_64.rpm` — `dnf install ./claude-crab-*.rpm`
- **macOS**: `claude-crab-<version>-macos.dmg` — a universal (arm64 + x86_64)
  app bundle. It is ad-hoc signed but not notarized, so the first launch needs
  right-click → Open.
- **Windows**: `claude-crab-<version>-setup.exe` — an installer with an
  optional start-at-sign-in task.

To rebuild the .deb and .rpm locally:

```sh
cargo build --release
bash packaging/stage-icons.sh
cargo deb --no-build      # target/debian/
cargo generate-rpm        # target/generate-rpm/
```

### Vendored dependency

`vendor/skia-rs-codec` is a `[patch.crates-io]` copy of the upstream crate with
`webp` dropped from its default features. Upstream pulls `libwebp-sys`, a C
library, which defeats a pure-Rust stack and breaks cross-compilation; Cargo
features are additive and nothing in the chain sets `default-features = false`,
so patching is the only way to drop it downstream. The app only ever decodes its
own PNG sheets. The proper fix belongs upstream.

## Flatpak

Every release carries a single-file bundle, which is how the Flatpak build
reaches anyone given Flathub does not accept AI-assisted applications:

```sh
flatpak install --user ./claude-crab-1.1.1.flatpak
```

The bundle records Flathub as its runtime repository, so a machine without
`org.kde.Platform` is offered it rather than failing on an unresolved
dependency. Build one yourself with `tools/build-bundle.sh`; a tag push builds
and attaches it automatically.

To build from source instead:

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

**Pinned dependencies.** Historically `layer-shell-qt` was not in `org.kde.Platform`, so the
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

`packaging/flathub/README.md` covers that and the remaining blockers, along with
how to run both halves of the linter.

Note that Flathub does not currently accept AI-assisted applications, and this
one was written with heavy AI assistance. The manifest is kept working and
linted, but a submission is not on the cards as things stand — see that file for
the policy text.

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
RUST_LOG=debug ./target/release/claude-crab
```

Startup logs the backend it picked, the inbox it is watching and the font it
found for the menu, which between them explain most "nothing happens" reports.

## How the signal gets here

```
claude session ──hook──▶ inbox/<ns>.json ──poll──▶ SessionTracker ──▶ Brain ──▶ Renderer
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

Right-clicking the character opens a menu for switching sprite variants and for
pinning it. Every choice is written to the config file straight away, so it
survives the restarts a systemd-managed service makes routine.

The crab always roams. Unpinned, it can be dragged with the left button to any
coordinate on the screen — the layer-shell surface covers the whole output,
with input confined to the character — and it resumes roaming from wherever it
is dropped, keeping that height until dragged again. **Pin** disables dragging
— the crab carries on roaming but ignores the left button — and **Unpin**
makes it draggable again. Pinning saves the crab's height to the config file
(in logical pixels, clamped to the screen on use), so a pinned crab comes back
at the same height after a restart. (The floating backend's window only spans the bottom
strip, so drags there stay within it.)

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
  "lockPosition": false,
  "lockedX": 0,
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
manifest is the contract — a hand-drawn replacement sheet needs no code change so
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

Clawd, the crab character depicted by the sprites, is a trademark of Anthropic,
PBC. This project is not affiliated with, sponsored by, or endorsed by
Anthropic; the MIT licence covers this project's code and generated art, not
the Clawd character or the Claude and Anthropic names.
