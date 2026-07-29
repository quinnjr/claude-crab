"""Tests for the hook installer.

Every test runs against a temporary XDG_DATA_HOME and HOME, so nothing here can
touch the real Claude Code configuration.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))

import crab_hooks  # noqa: E402


@pytest.fixture
def env(tmp_path, monkeypatch):
    home = tmp_path / "home"
    data = home / ".local" / "share"
    (data / "claude-profiles" / "personal").mkdir(parents=True)
    (data / "claude-profiles" / "work").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("XDG_DATA_HOME", str(data))
    monkeypatch.delenv("CLAUDE_HOME", raising=False)
    monkeypatch.delenv("CLAUDE_CONFIG_DIR", raising=False)
    monkeypatch.setattr(Path, "home", classmethod(lambda cls: home))
    return home


def settings_of(env_home: Path, profile: str) -> Path:
    return (
        env_home / ".local" / "share" / "claude-profiles" / profile / "settings.json"
    )


def read(path: Path) -> dict:
    return json.loads(path.read_text())


def run(*argv: str) -> int:
    return crab_hooks.main(list(argv))


# --- resolution ------------------------------------------------------------


def test_config_dir_wins_over_everything(env, monkeypatch):
    monkeypatch.setenv("CLAUDE_HOME", str(env / "from-env"))
    explicit = env / "explicit"
    assert run("--config-dir", str(explicit), "install") == crab_hooks.EXIT_OK
    assert (explicit / "settings.json").exists()
    assert not (env / "from-env").exists()


def test_profile_flag_resolves_under_profiles_root(env):
    run("--profile", "work", "install")
    assert settings_of(env, "work").exists()
    assert not settings_of(env, "personal").exists()


def test_explicit_all_beats_env(env, monkeypatch):
    monkeypatch.setenv("CLAUDE_HOME", str(env / "from-env"))
    run("--all", "install")
    assert settings_of(env, "work").exists()
    assert settings_of(env, "personal").exists()
    assert not (env / "from-env").exists()


def test_env_beats_the_default(env, monkeypatch):
    monkeypatch.setenv("CLAUDE_HOME", str(env / "from-env"))
    run("install")
    assert (env / "from-env" / "settings.json").exists()
    assert not (env / ".claude").exists()


def test_claude_home_beats_claude_config_dir(env, monkeypatch):
    monkeypatch.setenv("CLAUDE_HOME", str(env / "a"))
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(env / "b"))
    run("install")
    assert (env / "a" / "settings.json").exists()
    assert not (env / "b").exists()


def test_bare_invocation_targets_dot_claude(env):
    """The default is the one location every Claude Code install has. Profiles
    are opt-in through --all or --profile, not discovered behind the user's
    back."""
    run("install")
    assert (env / ".claude" / "settings.json").exists()
    for profile in ("personal", "work"):
        assert not settings_of(env, profile).exists()


def test_profiles_are_not_discovered_without_asking(env):
    """Even with profiles present and no environment override, a bare run must
    leave them alone."""
    run("install")
    assert not settings_of(env, "personal").exists()
    assert not settings_of(env, "work").exists()


def test_claude_config_dir_is_honoured_when_set(env, monkeypatch):
    """This is how a claude-profiles shell names its active profile."""
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(settings_of(env, "work").parent))
    run("install")
    assert settings_of(env, "work").exists()
    assert not (env / ".claude").exists()


def test_falls_back_to_dot_claude_when_no_profiles(tmp_path, monkeypatch):
    home = tmp_path / "home"
    (home / ".local" / "share").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("XDG_DATA_HOME", str(home / ".local" / "share"))
    monkeypatch.delenv("CLAUDE_HOME", raising=False)
    monkeypatch.delenv("CLAUDE_CONFIG_DIR", raising=False)
    monkeypatch.setattr(Path, "home", classmethod(lambda cls: home))
    run("install")
    assert (home / ".claude" / "settings.json").exists()


def test_flatpak_ignores_the_redirected_data_home(env, monkeypatch, tmp_path):
    """Inside a Flatpak, XDG_DATA_HOME points into the sandbox. Honouring it
    makes discovery resolve to an empty path and report phantom profiles
    instead of touching the real ones."""
    sandbox = tmp_path / "sandboxed-data"
    (sandbox / "claude-profiles" / "ghost").mkdir(parents=True)
    monkeypatch.setenv("XDG_DATA_HOME", str(sandbox))
    monkeypatch.setattr(crab_hooks, "inside_flatpak", lambda: True)

    found = [p.name for p in crab_hooks.discover_profiles()]
    assert found == ["personal", "work"]
    assert "ghost" not in found


def test_host_still_honours_xdg_data_home(env, monkeypatch, tmp_path):
    elsewhere = tmp_path / "elsewhere"
    (elsewhere / "claude-profiles" / "solo").mkdir(parents=True)
    monkeypatch.setenv("XDG_DATA_HOME", str(elsewhere))
    monkeypatch.setattr(crab_hooks, "inside_flatpak", lambda: False)

    assert [p.name for p in crab_hooks.discover_profiles()] == ["solo"]


# --- install ---------------------------------------------------------------


def test_install_registers_every_event(env):
    run("--profile", "work", "install")
    data = read(settings_of(env, "work"))
    assert set(data["hooks"]) == set(crab_hooks.EVENTS)
    for event in crab_hooks.EVENTS:
        entry = data["hooks"][event][0]["hooks"][0]
        assert entry["type"] == "command"
        assert crab_hooks.MARKER in entry["command"]


def test_tool_events_carry_a_matcher_and_others_do_not(env):
    run("--profile", "work", "install")
    hooks = read(settings_of(env, "work"))["hooks"]
    assert hooks["PreToolUse"][0]["matcher"] == crab_hooks.MATCHER
    assert hooks["PostToolUse"][0]["matcher"] == crab_hooks.MATCHER
    assert "matcher" not in hooks["Stop"][0]


def test_install_is_idempotent(env):
    path = settings_of(env, "work")
    run("--profile", "work", "install")
    first = path.read_text()
    assert run("--profile", "work", "install") == crab_hooks.EXIT_OK
    assert path.read_text() == first


def test_install_preserves_unrelated_keys_and_hooks(env):
    path = settings_of(env, "work")
    path.write_text(
        json.dumps(
            {
                "model": "opus",
                "hooks": {
                    "Stop": [{"hooks": [{"type": "command", "command": "notify-send hi"}]}]
                },
            }
        )
    )
    run("--profile", "work", "install")
    data = read(path)
    assert data["model"] == "opus"
    commands = [e["command"] for e in data["hooks"]["Stop"][0]["hooks"]]
    assert "notify-send hi" in commands
    assert any(crab_hooks.MARKER in c for c in commands)


def test_stale_entry_is_replaced_in_place_not_duplicated(env):
    path = settings_of(env, "work")
    old = f"echo old {crab_hooks.MARKER}"
    path.write_text(
        json.dumps(
            {
                "hooks": {
                    "Stop": [
                        {
                            "hooks": [
                                {"type": "command", "command": old},
                                {"type": "command", "command": "keep me"},
                            ]
                        }
                    ]
                }
            }
        )
    )
    run("--profile", "work", "install")
    entries = read(path)["hooks"]["Stop"][0]["hooks"]
    assert len(entries) == 2
    assert entries[0]["command"] == crab_hooks.COMMAND  # position preserved
    assert entries[1]["command"] == "keep me"


def test_status_reports_stale_before_reinstall(env):
    path = settings_of(env, "work")
    path.write_text(
        json.dumps(
            {
                "hooks": {
                    "Stop": [
                        {"hooks": [{"type": "command", "command": f"old {crab_hooks.MARKER}"}]}
                    ]
                }
            }
        )
    )
    data = read(path)
    states = crab_hooks.inspect(data)
    assert states["Stop"] == "stale"
    assert states["PreToolUse"] == "missing"


# --- uninstall -------------------------------------------------------------


def test_install_uninstall_round_trips_to_empty(env):
    path = settings_of(env, "work")
    path.write_text(json.dumps({"model": "opus"}))
    run("--profile", "work", "install")
    run("--profile", "work", "uninstall")
    assert read(path) == {"model": "opus"}


def test_round_trip_removes_hooks_key_that_did_not_exist_before(env):
    path = settings_of(env, "work")
    path.write_text(json.dumps({}))
    run("--profile", "work", "install")
    run("--profile", "work", "uninstall")
    assert "hooks" not in read(path)


def test_uninstall_leaves_unrelated_hooks_alone(env):
    path = settings_of(env, "work")
    path.write_text(
        json.dumps(
            {
                "hooks": {
                    "Stop": [{"hooks": [{"type": "command", "command": "notify-send hi"}]}]
                }
            }
        )
    )
    run("--profile", "work", "install")
    run("--profile", "work", "uninstall")
    data = read(path)
    assert data["hooks"]["Stop"][0]["hooks"] == [
        {"type": "command", "command": "notify-send hi"}
    ]


def test_uninstall_never_removes_an_unmarked_lookalike(env):
    """A hand-written hook that mentions the crab is not ours to delete."""
    path = settings_of(env, "work")
    handwritten = 'cat > "$HOME/.local/state/claude-crab/inbox/mine.json"'
    path.write_text(
        json.dumps(
            {"hooks": {"Stop": [{"hooks": [{"type": "command", "command": handwritten}]}]}}
        )
    )
    run("--profile", "work", "uninstall")
    assert read(path)["hooks"]["Stop"][0]["hooks"][0]["command"] == handwritten


def test_status_on_an_unreachable_dir_is_an_error_not_missing(env, capsys):
    """A directory the process cannot see looks exactly like one with no hooks
    installed. Reporting the latter would send someone hunting a phantom
    install problem when the real fault is a missing Flatpak grant."""
    code = run("--config-dir", str(env / "nowhere"), "status")
    assert code == crab_hooks.EXIT_ERROR
    err = capsys.readouterr().err
    assert "not an accessible directory" in err
    assert "missing" not in err


def test_unreachable_hint_mentions_the_grant_only_in_a_flatpak(env, monkeypatch, capsys):
    monkeypatch.setattr(crab_hooks, "inside_flatpak", lambda: True)
    run("--config-dir", str(env / "nowhere"), "status")
    assert "--filesystem=" in capsys.readouterr().err

    monkeypatch.setattr(crab_hooks, "inside_flatpak", lambda: False)
    run("--config-dir", str(env / "nowhere"), "status")
    assert "--filesystem=" not in capsys.readouterr().err


def test_install_may_still_create_a_missing_dir(env):
    """install is the one verb allowed to bring the directory into existence."""
    target = env / "brand-new"
    assert run("--config-dir", str(target), "install") == crab_hooks.EXIT_OK
    assert (target / "settings.json").exists()


def test_unwritable_target_reports_instead_of_tracebacking(env, capsys):
    target = env / "readonly"
    target.mkdir()
    (target / "settings.json").write_text("{}")
    target.chmod(0o500)
    try:
        code = run("--config-dir", str(target), "install")
        assert code == crab_hooks.EXIT_ERROR
        assert "cannot write" in capsys.readouterr().err
    finally:
        target.chmod(0o700)


def test_uninstall_on_missing_file_is_a_noop(env):
    assert run("--profile", "work", "uninstall") == crab_hooks.EXIT_OK
    assert not settings_of(env, "work").exists()


# --- safety ----------------------------------------------------------------


def test_invalid_json_is_skipped_and_other_targets_still_install(env, capsys):
    """--all, because the point is that one bad target does not stop the rest."""
    settings_of(env, "work").write_text("{ this is not json")
    code = run("--all", "install")
    assert code == crab_hooks.EXIT_ERROR
    assert settings_of(env, "personal").exists()
    assert settings_of(env, "work").read_text() == "{ this is not json"
    assert "not valid JSON" in capsys.readouterr().err


def test_install_takes_a_backup(env):
    path = settings_of(env, "work")
    path.write_text(json.dumps({"model": "opus"}))
    run("--profile", "work", "install")
    backups = list(path.parent.glob("settings.json.crab-backup-*"))
    assert len(backups) == 1
    assert read(backups[0]) == {"model": "opus"}


@pytest.mark.parametrize("command", ["install", "uninstall"])
def test_dry_run_writes_nothing(env, command, capsys):
    path = settings_of(env, "work")
    if command == "uninstall":
        run("--profile", "work", "install")
    before = path.read_text() if path.exists() else None

    run("--profile", "work", "--dry-run", command)

    after = path.read_text() if path.exists() else None
    assert after == before
    out = capsys.readouterr().out
    assert "@@" in out  # a unified diff was shown


def test_status_exit_code_tracks_completeness(env, capsys):
    assert run("--profile", "work", "status") == crab_hooks.EXIT_INCOMPLETE
    run("--profile", "work", "install")
    capsys.readouterr()
    assert run("--profile", "work", "status") == crab_hooks.EXIT_OK


def test_status_json_is_machine_readable(env, capsys):
    run("--profile", "work", "install")
    capsys.readouterr()
    run("--profile", "work", "--json", "status")
    report = json.loads(capsys.readouterr().out)
    assert report[0]["complete"] is True
    assert report[0]["events"]["PreToolUse"] == "installed"


def test_command_caps_payload_size():
    """tool_input can run to megabytes; the file that lands must not."""
    assert f'truncate -s "<{crab_hooks.HOOK_MAX_BYTES}"' in crab_hooks.COMMAND
    # head -c would close the pipe early and hand the writer an EPIPE.
    assert "head -c" not in crab_hooks.COMMAND


def test_command_caps_before_the_atomic_rename():
    """Truncating after the rename would expose an oversized file to the
    watcher, and truncating a file it is already reading."""
    cmd = crab_hooks.COMMAND
    assert cmd.index("truncate") < cmd.index("mv ")


def test_backups_are_pruned_to_a_bounded_history(env):
    path = settings_of(env, "work")
    path.write_text(json.dumps({"model": "opus"}))

    # Each install takes a backup; without pruning these accrue forever.
    for i in range(crab_hooks.MAX_BACKUPS + 4):
        path.write_text(json.dumps({"model": f"opus-{i}"}))
        crab_hooks.backup(path)

    backups = list(path.parent.glob("settings.json.crab-backup-*"))
    assert len(backups) == crab_hooks.MAX_BACKUPS


def test_pruning_keeps_the_newest_backups(env, monkeypatch):
    path = settings_of(env, "work")
    path.write_text("{}")

    for stamp in ("20200101-000000", "20990101-000000", "20500101-000000"):
        (path.parent / f"settings.json.crab-backup-{stamp}").write_text("{}")

    monkeypatch.setattr(crab_hooks, "MAX_BACKUPS", 2)
    crab_hooks.prune_backups(path)

    remaining = sorted(p.name for p in path.parent.glob("settings.json.crab-backup-*"))
    assert remaining == [
        "settings.json.crab-backup-20500101-000000",
        "settings.json.crab-backup-20990101-000000",
    ]


def test_command_sweeps_the_inbox_itself():
    """The crab prunes only while it is running, but the hooks fire regardless.
    Without a sweep here, a stopped crab lets the inbox grow without bound."""
    cmd = crab_hooks.COMMAND
    assert f"-mmin +{crab_hooks.HOOK_GC_AGE_MINUTES}" in cmd
    assert "-delete" in cmd


def test_sweep_runs_before_the_marker():
    """Anything after the marker is inside a shell comment and never runs, so
    a sweep appended there would be silently dead."""
    cmd = crab_hooks.COMMAND
    assert cmd.index("find") < cmd.index(crab_hooks.MARKER)


def test_sweep_is_sampled_not_unconditional():
    """A find on every tool call would put the cost of the sweep in the hot
    path; `case` is a builtin, so the unsampled writes fork nothing extra."""
    cmd = crab_hooks.COMMAND
    assert f'case "$f" in {crab_hooks.HOOK_GC_SAMPLE})' in cmd
    assert cmd.count("find") == 1
    assert cmd.index("case") < cmd.index("find") < cmd.index("esac")


def test_sweep_only_touches_files_in_the_inbox():
    """-maxdepth 1 and -type f keep the sweep off the directory itself and out
    of anything below it."""
    cmd = crab_hooks.COMMAND
    assert "-maxdepth 1" in cmd
    assert "-type f" in cmd
    assert 'find "$d"' in cmd


def test_write_still_happens_before_any_sweep():
    """The event is the point; the sweep is housekeeping and must never be able
    to preempt it."""
    cmd = crab_hooks.COMMAND
    assert cmd.index("mv ") < cmd.index("case")


def test_command_is_shell_safe_and_marked():
    assert crab_hooks.COMMAND.rstrip().endswith(crab_hooks.MARKER)
    # The marker must be a comment, not an argument, or every hook invocation
    # would try to run it.
    assert f" {crab_hooks.MARKER}" in crab_hooks.COMMAND
