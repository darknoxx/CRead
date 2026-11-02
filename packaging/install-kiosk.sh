#!/bin/sh
# Install CRead as the one program on a Raspberry Pi (or any small Debian box).
#
#   sudo ./install-kiosk.sh /path/to/cread
#
# The binary is user-owned so `SETTINGS -> UPDATE` can replace it without root;
# systemd owns the session and restarts cage + CRead together.
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "run this as root" >&2
    exit 1
fi

USER_NAME=${CREAD_USER:-cread}
VAULT=${CREAD_VAULT:-/home/$USER_NAME/Documents/CRead}
XKB_LAYOUT=${XKB_LAYOUT:-de}
BIN=${1:?usage: $0 /path/to/cread}
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

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

# A kiosk does not need a journal on the card. Keeps the SD alive.
install -d -m 0755 /etc/systemd/journald.conf.d
cat >/etc/systemd/journald.conf.d/cread-volatile.conf <<'EOF'
[Journal]
Storage=volatile
RuntimeMaxUse=32M
EOF

# Only disable the swap unit here. Stopping it runs swapoff, which can block
# for a long time on a small card; it will be gone after the next boot.
if systemctl list-unit-files dphys-swapfile.service >/dev/null 2>&1; then
    systemctl disable dphys-swapfile.service >/dev/null 2>&1 || true
fi
systemctl disable --now getty@tty1.service >/dev/null 2>&1 || true
# The compositor needs /run/user/<uid>; lingering creates it even before the
# first login session.
loginctl enable-linger "$USER_NAME" >/dev/null 2>&1 || true
install -m 0755 "$HERE/cread-update.sh" /usr/local/sbin/cread-update
systemctl restart systemd-journald
systemctl daemon-reload
systemctl enable cread-kiosk.service

cat <<EOF

CRead kiosk installed for $USER_NAME.

  binary:  /opt/cread/cread
  vault:   $VAULT
  service: cread-kiosk.service (starting now)

Open the WebDAV form once inside CRead (SETTINGS -> WEBDAV) and switch sync on.
The in-app updater replaces /opt/cread/cread and exits; systemd brings cage and
CRead back together.

Still worth doing by hand for the SD card:
  - add noatime to the root entry in /etc/fstab
  - quiet boot: quiet in /boot/firmware/cmdline.txt, disable_splash=1 in
    /boot/firmware/config.txt
  - do NOT use an overlay/read-only root: the in-app updater writes /opt/cread
EOF

# Start (or restart) the kiosk so the screen does not stay on a bare console.
if ! systemctl restart cread-kiosk.service; then
    echo "cread-kiosk did not start; check: journalctl -u cread-kiosk -b" >&2
fi
