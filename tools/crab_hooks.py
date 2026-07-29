#!/usr/bin/env python3
"""Install, remove, and report on claude-crab's Claude Code hook registration.

Every command this tool writes carries a trailing ``# claude-crab:v1`` shell
comment. The comment is inert to sh, survives reformatting, and is the only
thing ``uninstall`` and ``status`` match on -- a hand-written hook that happens
to mention the crab is never touched.

Stdlib only, so it can run from a PKGBUILD or a fresh checkout.
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import shutil
import sys
import time
from pathlib import Path
from typing import Any, Iterable

MARKER = "# claude-crab:v1"

# Mirrors inboxDir() in src/main.cpp. Both sides must agree, and both
# honour XDG_STATE_HOME so a non-default state root still works.
STATE_DIR = "${XDG_STATE_HOME:-$HOME/.local/state}/claude-crab/inbox"

# Payloads embed tool_input, which for a large Write or Edit can run to
# megabytes. The crab needs only session_id, hook_event_name and tool_name, and
# Claude Code emits all three before tool_input, so a byte cap keeps every field
# that matters. SessionTracker salvages those fields when a file is truncated.
HOOK_MAX_BYTES = 16384

# The crab prunes the inbox itself, but only while it is running. The hooks keep
# firing whether or not it is, so a stopped or uninstalled crab would otherwise
# let the directory grow without bound. This sweep is the safety net: it matches
# the daemon's default age limit and is deliberately a backstop, not the policy.
HOOK_GC_AGE_MINUTES = 60

# Sampled on the last digit of the nanosecond timestamp -- roughly one write in
# ten -- so the cost of the sweep is amortised instead of paid on every tool
# call. `case` is a shell builtin, so the other nine writes fork nothing extra.
HOOK_GC_SAMPLE = "*0"

# `truncate -s '<N'` shrinks to at most N and leaves smaller files alone. It is
# used in preference to `head -c N`, which would close the pipe early and hand
# the writing process an EPIPE for every oversized payload.
#
# Write to a temp name and rename, so the watcher never sees a partial file.
#
# The marker has to stay last: anything appended after it is inside the comment
# and silently never runs.
COMMAND = (
    f'd="{STATE_DIR}"; mkdir -p "$d"; f="$d/$(date +%s%N)"; '
    f'cat > "$f.tmp" && truncate -s "<{HOOK_MAX_BYTES}" "$f.tmp" && '
    f'mv "$f.tmp" "$f.json"; '
    f'case "$f" in {HOOK_GC_SAMPLE}) '
    f'find "$d" -maxdepth 1 -type f -mmin +{HOOK_GC_AGE_MINUTES} -delete 2>/dev/null;; '
    f'esac {MARKER}'
)

# Events the crab needs. PreToolUse/PostToolUse take a tool matcher; the rest
# are registered without one.
EVENTS = (
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
)
MATCHER_EVENTS = frozenset({"PreToolUse", "PostToolUse"})
MATCHER = "*"

EXIT_OK = 0
EXIT_INCOMPLETE = 1
EXIT_ERROR = 2


# --- config directory resolution -------------------------------------------


def profiles_root() -> Path:
    data_home = os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local" / "share")
    return Path(data_home) / "claude-profiles"


def discover_profiles() -> list[Path]:
    root = profiles_root()
    if not root.is_dir():
        return []
    return sorted(
        p for p in root.iterdir() if p.is_dir() and not p.name.startswith(".")
    )


def resolve_targets(args: argparse.Namespace) -> list[Path]:
    """Resolve which config directories to operate on.

    Precedence: --config-dir > --profile > --all > $CLAUDE_HOME >
    $CLAUDE_CONFIG_DIR > ~/.claude.

    --all is the default when no flag is given, because this machine keeps one
    config dir per profile and the selected profile is switchable. Patching only
    the active profile would leave the other profile's sessions invisible to the
    crab, which reads as a bug rather than a choice.
    """
    if args.config_dir:
        return [Path(args.config_dir).expanduser()]

    if args.profile:
        return [profiles_root() / args.profile]

    if args.all:
        found = discover_profiles()
        if found:
            return found
        # Fall through rather than silently doing nothing.

    for var in ("CLAUDE_HOME", "CLAUDE_CONFIG_DIR"):
        value = os.environ.get(var)
        if value:
            return [Path(value).expanduser()]

    if not args.all:
        found = discover_profiles()
        if found:
            return found

    return [Path.home() / ".claude"]


# --- settings.json I/O -----------------------------------------------------


class TargetError(Exception):
    """A single target could not be processed; others should still proceed."""


def settings_path(config_dir: Path) -> Path:
    return config_dir / "settings.json"


def load_settings(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    try:
        text = path.read_text()
    except OSError as exc:
        raise TargetError(f"cannot read {path}: {exc}") from exc
    if not text.strip():
        return {}
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        raise TargetError(f"{path} is not valid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise TargetError(f"{path} does not contain a JSON object")
    return data


def write_settings(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".claude-crab.tmp")
    with open(tmp, "w") as fh:
        fh.write(json.dumps(data, indent=2) + "\n")
        fh.flush()
        os.fsync(fh.fileno())
    os.replace(tmp, path)


# Backups are cheap but they are also forever; keep a useful history, not an
# unbounded one.
MAX_BACKUPS = 5


def prune_backups(path: Path) -> int:
    """Delete all but the newest MAX_BACKUPS backups of @p path."""
    existing = sorted(
        path.parent.glob(f"{path.name}.crab-backup-*"),
        key=lambda p: p.name,
        reverse=True,
    )
    removed = 0
    for stale in existing[MAX_BACKUPS:]:
        try:
            stale.unlink()
            removed += 1
        except OSError:
            pass
    return removed


def backup(path: Path) -> Path | None:
    if not path.exists():
        return None
    stamp = time.strftime("%Y%m%d-%H%M%S")
    dest = path.with_name(f"{path.name}.crab-backup-{stamp}")
    # Second resolution collides when two runs land in the same second, which
    # would silently overwrite the older backup -- the one more likely to hold
    # the pre-crab state worth recovering.
    suffix = 1
    while dest.exists():
        suffix += 1
        dest = path.with_name(f"{path.name}.crab-backup-{stamp}-{suffix}")
    shutil.copy2(path, dest)
    prune_backups(path)
    return dest


# --- hook tree surgery -----------------------------------------------------


def _groups(data: dict[str, Any], event: str) -> list[dict[str, Any]]:
    hooks = data.get("hooks")
    if not isinstance(hooks, dict):
        return []
    groups = hooks.get(event)
    if not isinstance(groups, list):
        return []
    return [g for g in groups if isinstance(g, dict)]


def _marked_entries(group: dict[str, Any]) -> list[dict[str, Any]]:
    entries = group.get("hooks")
    if not isinstance(entries, list):
        return []
    return [
        e
        for e in entries
        if isinstance(e, dict)
        and isinstance(e.get("command"), str)
        and MARKER in e["command"]
    ]


def _find_marked(data: dict[str, Any], event: str) -> dict[str, Any] | None:
    for group in _groups(data, event):
        marked = _marked_entries(group)
        if marked:
            return marked[0]
    return None


def inspect(data: dict[str, Any]) -> dict[str, str]:
    """Per-event state: 'installed', 'stale', or 'missing'."""
    result = {}
    for event in EVENTS:
        entry = _find_marked(data, event)
        if entry is None:
            result[event] = "missing"
        elif entry.get("command") == COMMAND:
            result[event] = "installed"
        else:
            result[event] = "stale"
    return result


def install(data: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    data = json.loads(json.dumps(data))  # deep copy, keeps the original intact
    changes: list[str] = []

    hooks = data.setdefault("hooks", {})
    if not isinstance(hooks, dict):
        raise TargetError("'hooks' exists but is not an object")

    for event in EVENTS:
        existing = _find_marked(data, event)
        if existing is not None:
            if existing.get("command") == COMMAND:
                continue
            # Replace in place, preserving position among sibling hooks.
            existing["command"] = COMMAND
            existing["type"] = "command"
            changes.append(f"{event}: replaced stale entry")
            continue

        groups = hooks.setdefault(event, [])
        if not isinstance(groups, list):
            raise TargetError(f"hooks.{event} exists but is not an array")

        wanted_matcher = MATCHER if event in MATCHER_EVENTS else None
        target_group = None
        for group in groups:
            if not isinstance(group, dict):
                continue
            if group.get("matcher") == wanted_matcher or (
                wanted_matcher is None and "matcher" not in group
            ):
                target_group = group
                break

        if target_group is None:
            target_group = {}
            if wanted_matcher is not None:
                target_group["matcher"] = wanted_matcher
            target_group["hooks"] = []
            groups.append(target_group)

        entries = target_group.setdefault("hooks", [])
        if not isinstance(entries, list):
            raise TargetError(f"hooks.{event}[].hooks is not an array")
        entries.append({"type": "command", "command": COMMAND})
        changes.append(f"{event}: added")

    return data, changes


def uninstall(data: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    data = json.loads(json.dumps(data))
    changes: list[str] = []

    hooks = data.get("hooks")
    if not isinstance(hooks, dict):
        return data, changes

    for event in list(hooks.keys()):
        groups = hooks.get(event)
        if not isinstance(groups, list):
            continue

        surviving_groups = []
        for group in groups:
            if not isinstance(group, dict):
                surviving_groups.append(group)
                continue
            entries = group.get("hooks")
            if isinstance(entries, list):
                kept = [
                    e
                    for e in entries
                    if not (
                        isinstance(e, dict)
                        and isinstance(e.get("command"), str)
                        and MARKER in e["command"]
                    )
                ]
                if len(kept) != len(entries):
                    changes.append(f"{event}: removed")
                    group["hooks"] = kept
            # Prune a group only if we emptied it and it holds nothing else.
            if group.get("hooks") == [] and set(group.keys()) <= {"matcher", "hooks"}:
                continue
            surviving_groups.append(group)

        if surviving_groups:
            hooks[event] = surviving_groups
        else:
            del hooks[event]

    if hooks == {}:
        del data["hooks"]

    return data, changes


# --- rendering -------------------------------------------------------------


def render(data: dict[str, Any]) -> str:
    return json.dumps(data, indent=2) + "\n"


def diff(before: dict[str, Any], after: dict[str, Any], path: Path) -> str:
    return "".join(
        difflib.unified_diff(
            render(before).splitlines(keepends=True),
            render(after).splitlines(keepends=True),
            fromfile=str(path),
            tofile=f"{path} (proposed)",
        )
    )


# --- commands --------------------------------------------------------------


def cmd_install(targets: Iterable[Path], args: argparse.Namespace) -> int:
    return _apply(targets, args, install, "install")


def cmd_uninstall(targets: Iterable[Path], args: argparse.Namespace) -> int:
    return _apply(targets, args, uninstall, "uninstall")


def _apply(targets, args, transform, verb: str) -> int:
    exit_code = EXIT_OK
    for config_dir in targets:
        path = settings_path(config_dir)
        try:
            before = load_settings(path)
            after, changes = transform(before)
        except TargetError as exc:
            print(f"error: {exc}", file=sys.stderr)
            exit_code = EXIT_ERROR
            continue

        if not changes:
            print(f"{config_dir}: already up to date")
            continue

        if args.dry_run:
            print(f"--- {config_dir}: would {verb} ---")
            print(diff(before, after, path), end="")
            continue

        saved = backup(path)
        write_settings(path, after)
        detail = f" (backup: {saved.name})" if saved else ""
        print(f"{config_dir}: {verb}ed {len(changes)} change(s){detail}")
        for change in changes:
            print(f"  {change}")

    return exit_code


def cmd_status(targets: Iterable[Path], args: argparse.Namespace) -> int:
    report = []
    exit_code = EXIT_OK

    for config_dir in targets:
        path = settings_path(config_dir)
        try:
            data = load_settings(path)
        except TargetError as exc:
            report.append({"configDir": str(config_dir), "error": str(exc)})
            exit_code = EXIT_ERROR
            continue

        states = inspect(data)
        complete = all(v == "installed" for v in states.values())
        if not complete and exit_code == EXIT_OK:
            exit_code = EXIT_INCOMPLETE
        report.append(
            {"configDir": str(config_dir), "complete": complete, "events": states}
        )

    if args.json:
        print(json.dumps(report, indent=2))
        return exit_code

    for entry in report:
        if "error" in entry:
            print(f"{entry['configDir']}: ERROR {entry['error']}")
            continue
        mark = "ok" if entry["complete"] else "incomplete"
        print(f"{entry['configDir']}: {mark}")
        for event, state in entry["events"].items():
            if state != "installed":
                print(f"  {event}: {state}")
    return exit_code


# --- entry point -----------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="crab_hooks", description=__doc__)
    parser.add_argument("--config-dir", help="operate on this config directory only")
    parser.add_argument("--profile", help="operate on this claude-profiles profile")
    parser.add_argument(
        "--all",
        action="store_true",
        help="operate on every discovered profile (the default)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="print a diff of the proposed change and write nothing",
    )
    parser.add_argument("--json", action="store_true", help="machine-readable status")

    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("install", help="register the crab's hooks")
    sub.add_parser("uninstall", help="remove only the hooks this tool owns")
    sub.add_parser("status", help="report per-target hook state")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    targets = resolve_targets(args)

    handlers = {
        "install": cmd_install,
        "uninstall": cmd_uninstall,
        "status": cmd_status,
    }
    return handlers[args.command](targets, args)


if __name__ == "__main__":
    sys.exit(main())
