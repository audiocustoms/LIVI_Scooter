#!/usr/bin/env bash
set -euo pipefail

# Enables the I2C bus an INA228 pack monitor sits on, and installs the tools to
# look for it. Run this on the Pi itself, not through the dev host.
#
#   sudo scripts/dev/livi-pi-i2c.sh              # enable, install, no reboot
#   sudo scripts/dev/livi-pi-i2c.sh --reboot     # and reboot afterwards
#   scripts/dev/livi-pi-i2c.sh --check           # only report what is set up
#
# The installer does the same through livi_enable_i2c in
# scripts/install/common.sh, so this is only needed on a host installed before
# that existed, or to inspect one.

BUS=1
TOOLS=i2c-tools
# The overlay creates the bus, but /dev/i2c-* only appears once i2c-dev is loaded.
MODULE=i2c-dev
MODULES_CONF=/etc/modules-load.d/livi-i2c.conf
say() { echo "→ $*"; }
die() { echo "Error: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
usage: livi-pi-i2c.sh [--reboot] [--check]

  --reboot   restart when the overlay was added, since it only takes effect then
  --check    report the current state and exit, changing nothing
EOF
}

reboot=0
check=0
while [ $# -gt 0 ]; do
  case "$1" in
    --reboot) reboot=1; shift ;;
    --check) check=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done

# Bookworm and later keep the boot files under /boot/firmware.
config=/boot/firmware/config.txt
[ -f "$config" ] || config=/boot/config.txt
[ -f "$config" ] || die "no config.txt found; is this a Raspberry Pi?"

has_overlay() { grep -Eq '^[[:space:]]*dtparam=i2c_arm=on' "$config"; }
has_tools() { command -v i2cdetect >/dev/null 2>&1 || [ -x /usr/sbin/i2cdetect ]; }
has_module() { [ -f "$MODULES_CONF" ] && grep -qxF "$MODULE" "$MODULES_CONF"; }
probe() { [ -x /usr/sbin/i2cdetect ] || return 1; /usr/sbin/i2cdetect -y "$BUS" "$1" "$1" 2>/dev/null | grep -qiE "^[0-9]{2}:.*[0-9a-f]{2}"; }

state() {
  echo "config.txt       : $config"
  echo "i2c_arm enabled  : $(has_overlay && echo yes || echo no)"
  echo "$MODULE loaded     : $(lsmod | grep -q "^$(echo "$MODULE" | tr - _)" && echo yes || echo no)"
  echo "$MODULES_CONF : $(has_module && echo listed || echo missing)"
  echo "/dev/i2c-$BUS       : $([ -e "/dev/i2c-$BUS" ] && echo present || echo missing)"
  echo "$TOOLS         : $(has_tools && echo installed || echo missing)"
  if [ -e "/dev/i2c-$BUS" ] && has_tools; then
    if probe 0x40; then
      echo "INA228 at 0x40  : present"
    else
      echo "INA228 at 0x40  : not answering"
    fi
  fi
}

if [ "$check" -eq 1 ]; then
  state
  exit 0
fi

[ "$(id -u)" -eq 0 ] || die "run as root: sudo $0 $*"

changed=0
if has_overlay; then
  say "i2c_arm is already enabled"
else
  say "enabling i2c_arm in $config"
  cp -p "$config" "$config.livi-bak"
  {
    echo ""
    echo "# Added by LIVI for the INA228 pack monitor."
    echo "dtparam=i2c_arm=on"
  } >>"$config"
  changed=1
fi

if has_tools; then
  say "$TOOLS is already installed"
else
  say "installing $TOOLS"
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends "$TOOLS"
fi

if has_module; then
  say "$MODULE is already listed in $MODULES_CONF"
else
  say "loading $MODULE at boot, so /dev/i2c-$BUS exists"
  mkdir -p "$(dirname "$MODULES_CONF")"
  echo "$MODULE" >"$MODULES_CONF"
fi
modprobe "$MODULE" || die "modprobe $MODULE failed"

state

if [ "$changed" -eq 1 ]; then
  if [ "$reboot" -eq 1 ]; then
    say "rebooting so the overlay takes effect"
    systemctl reboot
  else
    say "reboot for the overlay to take effect, then look for the chip with: i2cdetect -y $BUS"
  fi
fi
