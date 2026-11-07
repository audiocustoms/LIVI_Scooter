#!/usr/bin/env bash
set -euo pipefail

# Builds livi-ui incrementally and opens it in a window on the development host.
#
# Meant for WSL2 with WSLg: the checkout is synced into the Linux home, built
# there (fast, and off the Windows filesystem), and shown through WSLg. UI work
# needs no core, no AppImage and no deploy; the window runs on a fixture state.
#
#   scripts/dev/livi-window.sh                 # sync, build if stale, open
#   scripts/dev/livi-window.sh --page settings --settings car/carType
#   scripts/dev/livi-window.sh --rebuild       # force a build
#   scripts/dev/livi-window.sh --stop          # close the window

say() { echo "→ $*"; }
die() { echo "Error: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
usage: livi-window.sh [options]

  --repo PATH      checkout to build from (default: this script's repo)
  --dir PATH       Linux build directory (default: ~/livi)
  --page NAME      open settings, telemetry, projection or media (default: telemetry)
  --settings PATH  with --page settings, open this path, e.g. car/carType
  --size WxH       window size, e.g. 800x480
  --state FILE     core state to render (default: fixtures/scooter.json)
  --rebuild        build even when the binary looks up to date
  --no-sync        leave the build directory as it is
  --stop           close a running window and exit
EOF
}

repo="$(cd "$(dirname "$0")/../.." && pwd)"
build_dir="$HOME/livi"
page="telemetry"
settings=""
size=""
state="crates/livi-ui/fixtures/scooter.json"
rebuild=0
sync=1
stop=0

while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="${2:?--repo needs a path}"; shift 2 ;;
    --dir) build_dir="${2:?--dir needs a path}"; shift 2 ;;
    --page) page="${2:?--page needs a name}"; shift 2 ;;
    --settings) settings="${2:?--settings needs a path}"; shift 2 ;;
    --size) size="${2:?--size needs WxH}"; shift 2 ;;
    --state) state="${2:?--state needs a file}"; shift 2 ;;
    --rebuild) rebuild=1; shift ;;
    --no-sync) sync=0; shift ;;
    --stop) stop=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done

bin="$build_dir/native/target/release/LIVI"

if [ "$stop" -eq 1 ]; then
  if pkill -f "$bin"; then say "window closed"; else say "no window running"; fi
  exit 0
fi

[ "$(uname -s)" = Linux ] || die "run this inside WSL2 or Linux, not on Windows"

# WSLg exports both, but the Wayland socket is not always reachable; falling
# back to X11 is what actually shows a window there.
if [ -n "${WAYLAND_DISPLAY:-}" ] && [ ! -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/$WAYLAND_DISPLAY" ]; then
  unset WAYLAND_DISPLAY
fi
if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  die "no display found. Run inside WSL2 with WSLg (Windows 11, or Windows 10 21H2 and newer)"
fi

if [ "$sync" -eq 1 ]; then
  [ -d "$repo/native" ] || die "$repo does not look like the LIVI checkout"
  say "syncing $repo → $build_dir"
  mkdir -p "$build_dir"
  rsync -a --delete \
    --exclude '.git/' --exclude 'native/target/' --exclude 'android/' \
    --exclude 'docs/' --exclude '.github/' --exclude 'dist/' --exclude 'out/' \
    "$repo"/ "$build_dir"/
fi

export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null 2>&1 ||
  die "cargo not found. Install the toolchain with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"

stale=$rebuild
if [ ! -x "$bin" ]; then
  stale=1
elif [ "$stale" -eq 0 ] &&
  [ -n "$(find "$build_dir/native" -path '*/target' -prune -o \
    \( -name '*.rs' -o -name '*.slint' -o -name 'Cargo.toml' \) \
    -newer "$bin" -print -quit)" ]; then
  stale=1
fi

if [ "$stale" -eq 1 ]; then
  say "building livi-ui"
  ( cd "$build_dir/native" && cargo build --release -p livi-ui )
else
  say "livi-ui is up to date"
fi

[ -x "$bin" ] || die "$bin was not built"

if [ -z "$state" ]; then
  state="$build_dir/native/crates/livi-ui/fixtures/scooter.json"
elif [ "${state#/}" = "$state" ]; then
  state="$build_dir/native/$state"
fi
[ -f "$state" ] || die "no such state file: $state"

export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
export LIVI_UI_STATE="$state"
if [ -n "$page" ]; then export LIVI_UI_PAGE="$page"; fi
if [ -n "$settings" ]; then export LIVI_UI_SETTINGS="$settings"; fi
if [ -n "$size" ]; then export LIVI_UI_SIZE="$size"; fi

say "opening the window"
exec "$bin"
