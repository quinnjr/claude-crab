# Flathub submission notes

`dev.quinnjr.claude-crab.yml` here is the submission manifest. It differs from
`packaging/dev.quinnjr.claude-crab.yml` only in the `claude-crab` module's
source: this one builds from the tagged release, pinned to both tag and commit.

## Linting

```sh
flatpak install --user flathub org.flatpak.Builder
flatpak run --command=flatpak-builder-lint org.flatpak.Builder \
  manifest packaging/flathub/dev.quinnjr.claude-crab.yml

flatpak-builder --user --force-clean --repo=fhrepo \
  build-flathub packaging/flathub/dev.quinnjr.claude-crab.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo fhrepo
```

The manifest check and the repo check catch different things — the manifest
check passes clean while the repo check still reports the items below, so run
both.

## Outstanding before submitting

### 1. `desktop-file-is-nodisplay` — needs a linter exception

The desktop file sets `NoDisplay=true` deliberately. This is a background
companion with no window and no UI beyond the character itself: a launcher entry
would either do nothing visible, because the crab is already running, or start a
second one alongside the first.

Flathub treats `NoDisplay` as an error by default and grants exceptions for
background services. Request one for `dev.quinnjr.claude-crab` in the submission
pull request, citing the above.

The alternative — dropping `NoDisplay` — would put a menu entry in front of
users that promises an application that does not exist. That trade was
considered and rejected.

### 2. `metainfo-missing-screenshots`

The metainfo has no `<screenshots>`. Flathub requires at least one. The obvious
shot is the strip over a real panel, mid-walk, which has to be captured by hand.

### 3. `appstream-screenshots-not-mirrored-in-ostree`

Follows from 2 and resolves once screenshots are added and the build is
re-exported.

## Known non-blocking notes

- `runtime-update-available-to-org.kde.Platform-6.11` is informational. The
  manifest targets 6.9 because `layer-shell-qt` has to match the runtime's Qt:
  the 6.6 series onward needs Qt 6.10, while `org.kde.Sdk//6.9` carries 6.9.3,
  so the module is pinned to `layer-shell-qt` 6.5.5. Moving to a newer runtime
  means re-pinning that module in step.

## Permissions, and one deliberate omission

The Flatpak asks for the Wayland socket, an X11 fallback, IPC, DRI, and
`~/.local/state/claude-crab`. That last one is where Claude Code's hooks — which
run unsandboxed on the host — write their events.

It does **not** ask for access to Claude Code's `settings.json`, and should not.
Hook entries are arbitrary shell commands, so write access to that file amounts
to unsandboxed code execution on the host. Flatpak users install the hooks
host-side with `claude-crab-hooks` from the repository; the crab itself only
ever reads events out of the inbox.
