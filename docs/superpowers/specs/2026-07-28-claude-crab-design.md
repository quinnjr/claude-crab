# claude-crab — Design

**Date:** 2026-07-28
**Status:** Approved

Clawd walks in a strip just above the KDE Plasma panel, animating to reflect what
Claude Code is currently doing.

## Goals

- Glanceable, peripheral awareness of Claude Code activity without a popup or a panel widget.
- Unmistakable signal for the one state where the user is the blocker: Claude is waiting for input.
- Near-zero cost at rest, and no ability to destabilise the desktop shell.

## Non-goals (v1)

- Roaming across multiple monitors. One configured output only.
- Per-session crabs. A single crab reflects aggregate state.
- Dragging or repositioning the character by hand.
- A full settings UI. The right-click menu switches sprite variants and nothing
  else; everything else lives in the config file.

**Superseded.** The original design listed interaction as a non-goal, with the
window globally input-transparent. That was reversed to support the right-click
menu: input is now confined to the character's rectangle by a window mask, which
keeps the rest of the strip click-through while making the character itself
clickable.

## Environment

Target is the author's machine and anything close to it:

- Plasma 6.7.3, Wayland session
- Qt 6.11, `layer-shell-qt` and `libplasma` present
- Claude Code with hook support in `~/.claude/settings.json`

## Architecture

A standalone Qt6 C++ application with a QML UI. Explicitly **not** a plasmoid:
`layer-shell-qt` exposes no QML import, so a C++ entry point is required to configure
the layer surface. The independence is also a feature — a crash cannot take
`plasmashell` down, and QML can be hot-reloaded while tuning animation.

### Window

A single `QQuickWindow` promoted to a `wlr-layer-shell` surface:

- Layer: `Top`
- Anchors: bottom, left, right
- Height: configurable, default 72px
- Own exclusive zone: `0` (does not push other windows)
- Respects others' exclusive zones, so it sits *above* the panel rather than behind it
- `Qt::WindowTransparentForInput` — empty input region, clicks pass through
- `keyboardInteractivity: None`
- Transparent clear colour

X11 fallback: a frameless, always-on-top, input-transparent window positioned by
screen geometry. Degraded but functional.

### Signal path

```
claude session ──hook──▶ inbox/<ns>.json ──QFileSystemWatcher──▶ SessionTracker ──signal──▶ CrabBrain (QML)
```

State directory: `~/.local/state/claude-crab/inbox/`

The hook command is pure coreutils, to avoid interpreter or binary startup cost on
every single tool call:

```sh
tee ~/.local/state/claude-crab/inbox/$(date +%s%N).json >/dev/null
```

It writes the raw hook payload verbatim. `SessionTracker` parses `session_id` and
`hook_event_name` itself, then unlinks the file.

**Why one file per event rather than a shared append-only log:** `PreToolUse` payloads
embed `tool_input`, which for an `Edit` or `Write` routinely exceeds `PIPE_BUF`.
Appends above that size are not atomic, so concurrent sessions would interleave and
corrupt each other's records. Per-event files also give free ordering via the
nanosecond filename.

Hooks registered: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
`Notification`, `Stop`, `SessionEnd`.

## Components

Each is independently understandable and testable.

### 1. `tools/gen_sprites.py`

Renders the sprite sheets and `assets/manifest.json` from code. Pillow only.

The character is drawn the way the Claude Code mascot is drawn: flat terracotta
blocks on a coarse grid, no outline and no shading. A 12×8 cell body — torso, two
square eyes, a nub on each side, four stubby legs — on a 16×16 cell canvas at 4px
per cell, giving 64×64 frames, one animation per row.

Palette: body `#D06A4B`, eyes `#2B2A26`, monocle `#F0EEE6`.

Poses are expressed in grid cells rather than pixels; drawing off-grid is what
makes a blocky sprite look wrong. The few things that move by pixels (hops,
blinks, the lean on a fast gait) are named as such. Rotation snaps to quarter
turns, because nearest-neighbour rotation at an arbitrary angle shreds a blocky
sprite into loose pixels.

**Variants.** Two sheets are emitted from the same pose code:

| Variant | File | Look |
| --- | --- | --- |
| `default` | `spritesheet.png` | the plain character |
| `fancy` | `spritesheet-fancy.png` | top hat and monocle |

They share a single manifest, because they differ only in what is drawn inside a
frame, never in the layout. Both are compiled into the binary; the variant is
chosen at startup and costs nothing at runtime.

Manifest schema:

```json
{
  "frameWidth": 64,
  "frameHeight": 64,
  "animations": [
    { "name": "sleep",     "row": 0, "frames": 4,  "fps": 3,  "loop": true },
    { "name": "walk",      "row": 1, "frames": 8,  "fps": 10, "loop": true },
    { "name": "scuttle",   "row": 2, "frames": 8,  "fps": 18, "loop": true },
    { "name": "creep",     "row": 3, "frames": 8,  "fps": 5,  "loop": true },
    { "name": "think",     "row": 4, "frames": 6,  "fps": 5,  "loop": true },
    { "name": "wave",      "row": 5, "frames": 6,  "fps": 8,  "loop": true },
    { "name": "celebrate", "row": 6, "frames": 8,  "fps": 12, "loop": false },
    { "name": "tumble",    "row": 7, "frames": 10, "fps": 12, "loop": false }
  ]
}
```

The manifest is the contract between art and code. Replacing a PNG with
hand-drawn art requires no QML change so long as the layout matches.

The character is drawn facing right; leftward movement mirrors horizontally in
QML, which also keeps the monocle on the leading eye.

### 2. `src/SessionTracker.{h,cpp}`

The only non-trivial logic in the project, and deliberately pure C++ with no QML or
Wayland dependency so it can be unit-tested in isolation.

Responsibilities:

- Watch the inbox directory, read and unlink event files in filename order.
- Maintain `QHash<QString sessionId, SessionState>`.
- Compute and emit aggregate state.

Per-session state transitions:

| Hook event | Resulting session state |
| --- | --- |
| `SessionStart` | registered, `Idle` |
| `UserPromptSubmit` | `Working`, tool cleared |
| `PreToolUse` | `Working`, `currentTool = tool_name` |
| `PostToolUse` | `Working`, tool cleared; sets a transient error flag if the tool response indicates failure |
| `Notification` | `WaitingInput` |
| `Stop` | `Idle`, raises a one-shot `finished` event |
| `SessionEnd` | deregistered |

An event for an unknown `session_id` implicitly registers that session, so the crab
recovers correctly if it is started mid-session.

A sweep timer retires any session with no event for 10 minutes. Without this, a
`SIGKILL`ed session never emits `Stop` or `SessionEnd` and the crab walks forever.

Aggregate priority, highest first: `WaitingInput` → `Error` (transient, ~2s) →
`Working` → `Idle`.

Exposed to QML as properties: `aggregateState`, `currentTool`, and signals
`finished()` and `errored()`.

### 3. `src/main.cpp`

Layer-shell configuration, output selection, config loading, manifest loading, QML
engine setup, and the `--demo` / `--replay` / `--sprite` CLI options.

The manifest is parsed here rather than fetched from QML: it is a build artefact
compiled into the executable, so a failure is a packaging bug that must be
reported loudly, not swallowed by an async callback that may never fire.

### 4. `qml/Crab.qml`

An `AnimatedSprite` fed by the manifest. Exposes `play(name)` and a `facing`
property. Owns no policy — it only renders what it is told.

### 5. `qml/CrabBrain.qml`

Maps state to animation and drives movement. Picks walk targets, handles turning at
strip edges, and runs the corner-parking behaviour.

State → behaviour:

| Aggregate state | Behaviour |
| --- | --- |
| `Idle` | Walk to the configured corner, then `sleep` |
| `Working`, tool `Bash` | `scuttle` |
| `Working`, tool `Edit`/`Write`/`NotebookEdit` | `creep` |
| `Working`, tool `Read`/`Grep`/`Glob` | `walk` |
| `Working`, no tool | stop moving, `think` — planted, eyes drifting |
| `Working`, any other tool | `walk` |
| `WaitingInput` | stop, face viewer, `wave` on loop |
| `finished()` | `celebrate` once, then transition to `Idle` |
| `errored()` | `tumble` once, then resume prior behaviour |

### 6. `qml/main.qml`

The strip root. Positions the crab, applies scale, and hosts the brain.

### 7. `tools/crab_hooks.py`

A Python 3 CLI (stdlib only) that installs, removes, and reports on the hook
registration. Python rather than shell because the job is JSON surgery on a file the
user also edits by hand — merging into arbitrary existing structure, recognising its
own prior entries, and removing them without disturbing anything else is
meaningfully more than `jq` glue.

Subcommands:

| Command | Behaviour |
| --- | --- |
| `install` | Merge the seven hook entries into every resolved target |
| `uninstall` | Remove only the entries this tool owns |
| `status` | Report, per target, which hooks are present, stale, or missing; exit non-zero if any target is not fully installed |

Global flags: `--config-dir`, `--profile`, `--all`, `--dry-run`, `--json`.
`--dry-run` prints a unified diff of the proposed `settings.json` change and writes
nothing. It applies to `install` and `uninstall` alike.

**Ownership marker.** Every command this tool writes ends with a trailing
`# claude-crab:v1` shell comment. The comment is inert to `sh`, survives the user
reordering or reformatting the file, and is what `uninstall` and `status` match on.
Nothing is ever removed that does not carry the marker — a hand-written hook that
happens to reference the crab's state directory is left strictly alone.

**Merge semantics.** For each of the seven events, the tool looks for an existing
marked entry:

- No marked entry → append ours to that event's matcher group, preserving any
  unrelated hooks already registered there.
- Marked entry with an identical command → no change; the target is already current.
- Marked entry with a *different* command (an older version, or a hand-edit) →
  replace it in place, keeping its position in the list. `status` reports this
  pre-state as `stale`.

**Removal semantics.** `uninstall` drops marked entries, then prunes any matcher
group and any event key left empty, then removes the top-level `hooks` key if it
ends up empty. The file is left as close to its pre-install shape as the tool can
determine.

**Safety.** Writes are atomic — write to a sibling temp file, `fsync`, `os.replace`.
A timestamped backup is taken before the first modification of each target. A target
whose `settings.json` is not valid JSON is reported as an error and skipped; other
targets still proceed, and the process exits non-zero. A missing `settings.json` is
created on `install` and is a no-op on `uninstall`.

**Config directory resolution.** The machine uses `claude-profiles`, which keeps one
config directory per profile under `$XDG_DATA_HOME/claude-profiles/<name>` and
exports `CLAUDE_CONFIG_DIR` from a shell wrapper. The installer resolves targets in
this order:

1. `--config-dir <path>` — explicit, wins over everything.
2. `--profile <name>` — resolves to `$XDG_DATA_HOME/claude-profiles/<name>`.
3. `--all`, passed explicitly — every profile directory under
   `$XDG_DATA_HOME/claude-profiles/`.
4. `$CLAUDE_HOME`, then `$CLAUDE_CONFIG_DIR`, if either is set in the environment.
5. Auto-discovered profiles — **this is what a bare invocation does**, and it is
   equivalent to `--all`.
6. `~/.claude`.

Environment variables sit above auto-discovery but below an explicit `--all`: a set
`CLAUDE_HOME` is a deliberate signal about which config is in play, while a bare
invocation has no such signal and should cover everything.

`CLAUDE_HOME` is checked first among the environment variables even though
`claude-profiles` currently exports `CLAUDE_CONFIG_DIR`, so an explicit override
keeps working if the wrapper's variable name changes.

**Defaulting to `--all` is deliberate.** There are two profiles on this machine
(`personal` and `work`) and the profile default is switchable. Installing into only
the currently-selected profile would leave sessions from the other profile invisible
to the crab, which reads as a bug rather than as a configuration choice. All profiles
write into the same state directory; the crab neither knows nor cares which profile
an event came from.

The installer prints each target it patched, and skips — without failing — any
target already carrying the hooks.

### 8. `claude-crab.service`

A systemd `--user` unit, `WantedBy=graphical-session.target`.

## Configuration

`~/.config/claude-crab.json`, all keys optional:

| Key | Default | Meaning |
| --- | --- | --- |
| `stripHeight` | `72` | Height of the layer surface in px |
| `crabScale` | `1.0` | Sprite scale multiplier |
| `output` | first screen | Connector name, e.g. `DP-1` |
| `sleepCorner` | `"right"` | `"left"` or `"right"` |
| `sprite` | `"default"` | `"default"` or `"fancy"` (top hat and monocle) |
| `menuHeadroom` | `220` | Transparent space above the strip for the right-click menu |
| `inboxMaxAgeMinutes` | `60` | Events older than this are pruned |
| `inboxMaxMegabytes` | `32` | Inbox byte budget, oldest dropped first |
| `staleTimeoutMinutes` | `10` | Session retirement threshold |
| `reactions` | all `true` | Per-reaction toggles: `waiting`, `finished`, `error`, `toolFlavour` |

## Error handling

| Condition | Behaviour |
| --- | --- |
| Malformed JSON in inbox | Log at warning, unlink the file, continue. Never fatal. |
| Inbox flood | Process at most 200 files per tick, oldest first; drop the excess with a single warning. |
| Session goes silent | Retired by the sweep timer after `staleTimeoutMinutes`. |
| Layer-shell unavailable | Fall back to a frameless always-on-top window; log the downgrade. |
| Sprite sheet missing or manifest mismatch | Render a magenta box and log at error, so the failure is visible rather than an invisible crab. |
| Configured output absent | Fall back to the primary screen; log. |
| State directory missing | Create it on startup. |

## Testing

**Unit (QTest, `tests/tst_sessiontracker.cpp`)** — the real coverage lives here,
against recorded hook payload fixtures:

- Single session happy path through every hook event.
- Two and four sessions interleaved; aggregate reflects the correct priority.
- Out-of-order and duplicate events.
- Events for an unregistered session id.
- Stale sweep retires a silent session and updates the aggregate.
- Malformed JSON, empty file, and truncated file are survived.

**Sprite generation (`tests/test_gen_sprites.py`)** — asserts the generated sheet
dimensions match the manifest, every declared row exists, and no frame is fully
transparent.

**Installer (`tests/test_crab_hooks.py`, pytest against a temp `XDG_DATA_HOME`)** —

- Resolution precedence: `--config-dir` > `--profile` > `--all` > `CLAUDE_HOME` >
  `CLAUDE_CONFIG_DIR` > `~/.claude`.
- Default run patches every discovered profile, not just the selected one.
- `install` twice is a no-op and exits zero.
- `install` → `uninstall` round-trips to a file semantically equal to the original,
  including the case where `hooks` did not exist beforehand.
- Unrelated existing hooks on the same events survive both install and uninstall.
- An unmarked hand-written hook referencing the crab is never removed.
- A stale marked entry is replaced in place, not duplicated, and `status` reports it
  as stale beforehand.
- Invalid JSON in one target is reported and skipped while other targets still
  install; exit code is non-zero.
- `--dry-run` writes nothing for both `install` and `uninstall`.
- `status` exit code is non-zero when any target is incompletely installed.

**Manual / visual:**

- `claude-crab --demo` cycles every animation on a timer for art tuning.
- `claude-crab --replay fixtures/session.jsonl` deterministically replays a recorded
  real session, exercising the full state machine and every animation end to end.

## Build and packaging

CMake, Qt 6.11, `LayerShellQt` via `find_package`. `ninja` build. Arch `PKGBUILD`
included. The sprite sheet is generated at build time by `gen_sprites.py` and is
**not** checked in, so the script stays the source of truth.

## Open decisions, resolved

- Repository lives at `/home/joseph/Projects/claude-crab`.
- Multi-monitor roaming is out of scope for v1; one configured output.
- The crab art is an original design in Claude's palette. There is no official
  Anthropic crab asset available, and none is assumed.
