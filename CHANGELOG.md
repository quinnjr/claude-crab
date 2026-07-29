# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] — 2026-07-28

First release.

### Added

- A click-through layer-shell strip anchored above the KDE Plasma panel, with
  Clawd walking in it and animating to match Claude Code's activity.
- `SessionTracker`, folding hook events from any number of concurrent sessions
  into one aggregate state: waiting for input beats working beats idle.
- Distinct reactions for the states that matter — a wave when Claude is waiting
  on you, a celebration on finishing, a tumble on a failed tool — plus a gait
  that varies with the tool in use.
- A right-click menu on the character for switching sprite variants, persisted
  to the config file.
- Two sprite variants, `default` and `fancy` (top hat and monocle), generated
  from a cell grid at build time and sharing a single manifest.
- `claude-crab-hooks`, which installs, removes and reports on the hook
  registration across every `claude-profiles` profile, touching only entries it
  owns and keeping a bounded backup history.
- A systemd user unit, an AppStream component, hicolor icons rendered from the
  same code as the sprite, an Arch `PKGBUILD`, and a Flatpak manifest.
- `--demo` and `--replay` modes for exercising every animation without running
  Claude Code.

### Notes

- Wayland only in practice. There is an X11 fallback window, but layer-shell has
  no X11 equivalent, so it is a degraded path.
- One output per instance; roaming across monitors is not implemented.
- The hooks write events whether or not the crab is running, so each payload is
  capped at 16 KiB and the inbox is pruned by age and total size. Truncated
  payloads still yield a usable event, because Claude Code emits `session_id`,
  `hook_event_name` and `tool_name` ahead of `tool_input`.
- Inside a Flatpak, `XDG_STATE_HOME` points into the sandbox while the hooks
  write to the host's. `CrabConfig::inboxDir()` accounts for this;
  `$CLAUDE_CRAB_STATE_DIR` overrides it.

[1.0.0]: https://github.com/quinnjr/claude-crab/releases/tag/v1.0.0
