# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.0.1] — 2026-08-19

### Added

- Pinning saves the crab's height as `pinnedLift` (logical pixels above the
  strip floor), so a pinned crab comes back at the same height after a
  restart. The height is clamped to the screen on use.

### Fixed

- The release-bundle workflow failed on the v2.0.0 tag: the Flatpak manifest
  references `packaging/cargo-sources.json` for offline crate fetching, but
  the file was never generated and committed. It is now checked in
  (regenerate with `flatpak-cargo-generator.py Cargo.lock -o
  packaging/cargo-sources.json` whenever `Cargo.lock` changes).
- `tools/build-bundle.sh` still read the bundle version out of the deleted
  `CMakeLists.txt`; it now reads `Cargo.toml`.

## [2.0.0] — 2026-08-19

### Changed

- **Complete rewrite in Rust.** Qt and QML are gone; rendering is pure Rust
  (skia-rs) and the build is `cargo`, not CMake. Two window backends: a
  wlr-layer-shell surface on Wayland compositors that support it, and a
  floating always-on-top window (winit/wgpu) that brings the crab to macOS
  and Windows.
- The layer-shell surface covers the whole output rather than a bottom
  strip, with input still confined to the character, so a drag can drop the
  crab at any coordinate on the screen. It roams horizontally at the height
  it is dropped until dragged again.
- The right-click menu's lock toggle is now **Pin**/**Unpin** and only
  guards against dragging: a pinned crab roams as usual but ignores the
  left button.
- **Breaking config change.** `lockedX` is gone; `lockPosition` still
  persists the pin toggle, but a pinned position is no longer saved.

### Added

- A single-file `.flatpak` bundle attached to each release, built by
  `tools/build-bundle.sh` and by a workflow on tag push. It records Flathub as
  its runtime repository so a machine without `org.kde.Platform` is offered it
  rather than failing on an unresolved dependency.

## [1.1.1] — 2026-07-29

### Added

- README instructions for installing the hooks from inside the Flatpak,
  including that `flatpak run` passes the environment through — so a
  `CLAUDE_CONFIG_DIR` exported by the launching shell outranks the default
  unless `--unset-env` is passed.

### Changed

- **Behaviour change.** `claude-crab-hooks` now defaults to `~/.claude`, the one location every Claude
  Code install has. Previously a bare invocation discovered and patched every
  profile under `$XDG_DATA_HOME/claude-profiles/`; it no longer does.
  `$CLAUDE_HOME` and `$CLAUDE_CONFIG_DIR` still take precedence when set, and
  multi-profile setups opt in with `--all` or `--profile`.

### Fixed

- `claude-crab-hooks` reported every hook as missing when it could not read the
  config directory at all — indistinguishable from a clean install, and exactly
  what happens inside a Flatpak without the matching `--filesystem` grant. It
  now fails with an actionable message naming the path.
- Profile discovery honoured the sandbox's redirected `XDG_DATA_HOME` inside a
  Flatpak, resolving to an empty path and reporting phantom profiles.
- An unwritable target produced a traceback instead of an error.

## [1.1.0] — 2026-07-29

### Added

- A `party` sprite variant wearing a birthday hat, selectable from the
  right-click menu alongside the existing two.
- A Flathub submission manifest under `packaging/flathub/`, building from the
  tagged release, with notes on the linter exceptions it still needs.

### Fixed

- The inbox could grow without bound whenever the crab was not running: it was
  the only thing pruning, while the hooks write regardless. The hook now carries
  its own amortised sweep, so a stopped or uninstalled crab no longer leaves
  events piling up.

### Changed

- The Flatpak no longer requests access to Claude Code's `settings.json`. Hook
  entries are arbitrary shell commands, so write access there amounts to
  unsandboxed code execution on the host. Hooks are installed host-side
  instead; the crab only ever reads events.
- The Flatpak grants `fallback-x11`, so it starts in an X11 session and degrades
  to the frameless always-on-top window rather than failing to launch.

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

[2.0.1]: https://github.com/quinnjr/claude-crab/releases/tag/v2.0.1
[2.0.0]: https://github.com/quinnjr/claude-crab/releases/tag/v2.0.0
[1.1.1]: https://github.com/quinnjr/claude-crab/releases/tag/v1.1.1
[1.1.0]: https://github.com/quinnjr/claude-crab/releases/tag/v1.1.0
[1.0.0]: https://github.com/quinnjr/claude-crab/releases/tag/v1.0.0
