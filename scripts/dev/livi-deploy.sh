#!/usr/bin/env bash
set -euo pipefail

# Cross-builds the arm64 binaries in WSL and deploys them to a running LIVI host.
#
# The host keeps the resources of its installed AppImage (GStreamer bundle,
# icons, templates); only the binaries are replaced, so nothing has to be
# reinstalled. The kiosk restarts afterwards and shows the change.
#
#   scripts/dev/livi-deploy.sh                    # build what changed, deploy, restart
#   scripts/dev/livi-deploy.sh --host 192.168.0.128
#   scripts/dev/livi-deploy.sh --rebuild
#   scripts/dev/livi-deploy.sh --no-restart

say() { echo "→ $*"; }
die() { echo "Error: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
usage: livi-deploy.sh [options]

  --host HOST      ssh host of the device (default: chopper)
  --dir PATH       Linux build directory (default: ~/livi)
  --app PATH       install path on the device (default: ~/LIVI/app)
  --rebuild        build even when the binaries look up to date
  --no-sync        leave the build directory as it is
  --no-restart     deploy the binaries but leave the kiosk running
EOF
}

build_dir="$HOME/livi"
host=chopper
app='$HOME/LIVI/app'
rebuild=0
sync=1
restart=1

while [ $# -gt 0 ]; do
  case "$1" in
    --host) host="${2:?--host needs a host}"; shift 2 ;;
    --dir) build_dir="${2:?--dir needs a path}"; shift 2 ;;
    --app) app="$2"; shift 2 ;;
    --rebuild) rebuild=1; shift ;;
    --no-sync) sync=0; shift ;;
    --no-restart) restart=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done

[ "$(uname -s)" = Linux ] || die "run this inside WSL2 or Linux, not on Windows"
command -v aarch64-linux-gnu-gcc >/dev/null 2>&1 ||
  die "no aarch64 cross compiler. Install gcc-aarch64-linux-gnu and the :arm64 dev libraries"

if [ "$sync" -eq 1 ]; then
  repo="$(cd "$(dirname "$0")/../.." && pwd)"
  say "syncing $repo → $build_dir"
  mkdir -p "$build_dir"
  rsync -a --delete \
    --exclude '.git/' --exclude 'native/target/' --exclude 'android/' \
    --exclude 'docs/' --exclude '.github/' --exclude 'dist/' --exclude 'out/' \
    "$repo"/ "$build_dir"/
fi

export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null 2>&1 || die "cargo not found; install rustup first"

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig
export LIVI_VERSION="${LIVI_VERSION:-10.5.0}"
export LIVI_BUILD_SHA="${LIVI_BUILD_SHA:-$(date +%Y%m%d-%H%M)}"
export LIVI_BUILD_DATE="${LIVI_BUILD_DATE:-$(date +%F)}"

out="$build_dir/native/target/aarch64-unknown-linux-gnu/release"
stale=$rebuild
if [ "$stale" -eq 0 ] && [ -x "$out/LIVI" ]; then
  if [ -n "$(find "$build_dir/native" -path '*/target' -prune -o \
    \( -name '*.rs' -o -name '*.slint' -o -name 'Cargo.toml' \) \
    -newer "$out/LIVI" -print -quit)" ]; then
    stale=1
  fi
fi

if [ "$stale" -eq 1 ]; then
  say "building for aarch64"
  ( cd "$build_dir/native" && cargo build --release --target aarch64-unknown-linux-gnu \
    -p livi-ui -p livi-core -p livi-helperd -p livi-gst-host -p livi-compositor )
else
  say "aarch64 binaries are up to date"
fi

for bin in LIVI livi-core livi-helperd livi-gst-host livi-compositor; do
  [ -x "$out/$bin" ] || die "$out/$bin was not built"
done

say "deploying to $host"
# rsync does not expand $HOME on the far side, so resolve it once up front, and
# BatchMode keeps it from waiting for a password prompt that never comes.
if [ "${app#\$HOME}" != "$app" ]; then
  remote_home=$(ssh -o BatchMode=yes "$host" 'printf %s "$HOME"') ||
    die "cannot reach $host over ssh"
  app="${app#\$HOME}"
  app="$remote_home$app"
fi
ssh_cmd="ssh -o BatchMode=yes"

# One rsync per binary: --checksum keeps unchanged ones from being rewritten,
# which keeps the kiosk restart cheap.
rsync -a --checksum -e "$ssh_cmd" "$out/LIVI" "$host:$app/livi-ui"
rsync -a --checksum -e "$ssh_cmd" "$out/livi-core" "$host:$app/resources/core/livi-core"
rsync -a --checksum -e "$ssh_cmd" "$out/livi-helperd" "$host:$app/resources/driver/livi-helperd"
rsync -a --checksum -e "$ssh_cmd" "$out/livi-gst-host" "$host:$app/resources/gst-host/livi-gst-host"
rsync -a --checksum -e "$ssh_cmd" "$out/livi-compositor" "$host:$app/resources/compositor/livi-compositor"

ssh -o BatchMode=yes "$host" "chmod +x $app/livi-ui $app/resources/core/livi-core \
  $app/resources/driver/livi-helperd $app/resources/gst-host/livi-gst-host \
  $app/resources/compositor/livi-compositor"

if [ "$restart" -eq 1 ]; then
  say "restarting the kiosk"
  # Not "systemctl restart": ExecStopPost starts getty@tty1 with --no-block and
  # the unit conflicts with it, which makes systemd cancel the start half of the
  # restart transaction. Stop and start separately, letting the stop settle.
  ssh -o BatchMode=yes "$host" 'sudo -n systemctl stop livi-kiosk.service; sleep 2; sudo -n systemctl start livi-kiosk.service'
  sleep 4
  state=$(ssh -o BatchMode=yes "$host" 'systemctl is-active livi-kiosk.service')
  [ "$state" = active ] || die "livi-kiosk is $state after the restart"
  say "livi-kiosk is running"
fi

say "done"
