#!/usr/bin/env bash
# Install CRead as the one program on a Raspberry Pi (or any small Debian box).
#
#   sudo CREAD_USER=$USER ./install-kiosk.sh /path/to/cread
#
# The binary is user-owned so `SETTINGS -> UPDATE` can replace it without root;
# systemd owns the session and restarts cage + CRead together.
#
# Without CREAD_USER the user who invoked sudo is used; a bare root shell falls
# back to a dedicated `cread` account.
set -Eeuo pipefail
trap 'echo "install failed at line $LINENO" >&2' ERR

if [ "$(id -u)" -ne 0 ]; then
    echo "run this as root" >&2
    exit 1
fi

USER_NAME=${CREAD_USER:-${SUDO_USER:-cread}}
if [ "$USER_NAME" = "root" ]; then
    USER_NAME=cread
fi
VAULT=${CREAD_VAULT:-/home/$USER_NAME/Documents/CRead}
XKB_LAYOUT=${XKB_LAYOUT:-$(sed -n 's/^XKBLAYOUT="\(.*\)"/\1/p' /etc/default/keyboard 2>/dev/null | head -n 1)}
[ -n "$XKB_LAYOUT" ] || XKB_LAYOUT=us
BIN=${1:?usage: $0 /path/to/cread}
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

[ -x "$BIN" ] || {
    echo "$BIN is not an executable" >&2
    exit 1
}

if ! id "$USER_NAME" >/dev/null 2>&1; then
    useradd --create-home --shell /bin/bash "$USER_NAME"
fi
UID_N=$(id -u "$USER_NAME")

install -d -m 0755 /opt/cread
install -m 0755 "$BIN" /opt/cread/cread
chown -R "$USER_NAME:$USER_NAME" /opt/cread

# logind usually grants seat/GPU/audio access to the active session; these
# groups are the belt-and-braces version for pi users without a seat.
for group in video render audio input; do
    if getent group "$group" >/dev/null; then
        usermod -aG "$group" "$USER_NAME"
    fi
done

# A readable vault for the first frame; CRead fills the three rooms itself.
runuser -u "$USER_NAME" -- mkdir -p "$VAULT"

sed \
    -e "s|@USER@|$USER_NAME|g" \
    -e "s|@UID@|$UID_N|g" \
    -e "s|@VAULT@|$VAULT|g" \
    -e "s|@XKB@|$XKB_LAYOUT|g" \
    "$HERE/cread-kiosk.service.in" >/etc/systemd/system/cread-kiosk.service

install -d -m 0755 /etc/polkit-1/rules.d
sed -e "s|@USER@|$USER_NAME|g" \
    "$HERE/50-cread-poweroff.rules.in" >/etc/polkit-1/rules.d/50-cread-poweroff.rules

# `backtolinux` stops the kiosk and starts a login on tty1. The command runs
# as the kiosk user; the unit and its helper do the privileged part.
install -m 0755 "$HERE/cread-backtolinux.sh" /usr/local/sbin/cread-backtolinux
install -m 0644 "$HERE/cread-backtolinux.service" /etc/systemd/system/cread-backtolinux.service

# A kiosk does not need a journal on the card. Keeps the SD alive.
install -d -m 0755 /etc/systemd/journald.conf.d
cat >/etc/systemd/journald.conf.d/cread-volatile.conf <<'EOF'
[Journal]
Storage=volatile
RuntimeMaxUse=32M
EOF

# The essential step first: the unit must be enabled even if a later,
# optional tweak fails.
systemctl daemon-reload
systemctl enable cread-kiosk.service

# Optional steps. None of them may stop the installation or leave the kiosk
# disabled.
if systemctl list-unit-files dphys-swapfile.service >/dev/null 2>&1; then
    # Only disable the swap unit. Stopping it runs swapoff, which can block on
    # a small card; it will be gone after the next boot.
    systemctl disable dphys-swapfile.service >/dev/null 2>&1 || true
fi
# Do not stop the getty with --now: the kiosk's Conflicts= stops it at start.
systemctl disable getty@tty1.service >/dev/null 2>&1 || true
# The compositor needs /run/user/<uid>; lingering creates it at boot.
loginctl enable-linger "$USER_NAME" >/dev/null 2>&1 \
    || echo "warning: could not enable linger for $USER_NAME" >&2
if ! install -d -m 0755 /usr/local/sbin; then
    echo "warning: cannot create /usr/local/sbin" >&2
elif ! install -m 0755 "$HERE/cread-update.sh" /usr/local/sbin/cread-update; then
    echo "warning: cannot install cread-update; in-app updates still work" >&2
fi
systemctl restart systemd-journald >/dev/null 2>&1 || true

# Start and verify. A failure here must be visible to the caller.
if ! systemctl restart cread-kiosk.service; then
    echo "warning: cread-kiosk did not start cleanly" >&2
fi
for _ in $(seq 1 15); do
    if systemctl is-active --quiet cread-kiosk.service; then
        break
    fi
    sleep 1
done
if ! systemctl is-enabled --quiet cread-kiosk.service; then
    echo "error: cread-kiosk is not enabled" >&2
    exit 1
fi
if ! systemctl is-active --quiet cread-kiosk.service; then
    echo "error: cread-kiosk is not running; check: journalctl -u cread-kiosk -b" >&2
    systemctl status cread-kiosk.service --no-pager >&2 || true
    exit 1
fi

cat <<EOF

CRead kiosk installed for $USER_NAME.

  binary:  /opt/cread/cread
  vault:   $VAULT
  service: cread-kiosk.service (running)

Open the WebDAV form once inside CRead (SETTINGS -> WEBDAV) and switch sync on.
The in-app updater replaces /opt/cread/cread and exits; systemd brings cage and
CRead back together.

Still worth doing by hand for the SD card:
  - add noatime to the root entry in /etc/fstab
  - quiet boot: quiet in /boot/firmware/cmdline.txt, disable_splash=1 in
    /boot/firmware/config.txt
  - do NOT use an overlay/read-only root: the in-app updater writes /opt/cread
EOF
