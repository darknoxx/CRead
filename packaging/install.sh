#!/usr/bin/env bash
# Install the CRead kiosk on a fresh Debian / Raspberry Pi OS Lite system.
#
#   curl -fLO https://github.com/darknoxx/CRead/releases/latest/download/install.sh
#   less install.sh
#   sudo bash install.sh
#
# Download the release and read this file before running it. The release
# tarball is checked against its checksum and a minisign signature; the public
# keys are baked into this script so a swapped download cannot swap the key.
set -euo pipefail

VERSION=""
USER_NAME="${CREAD_USER:-}"
ASSUME_YES=0

usage() {
    cat <<'EOF'
Usage: sudo bash install.sh [options]

  --version VERSION   install a specific release (default: latest)
  --user NAME         the user that owns the kiosk and the vault
  --yes, -y           do not ask for confirmation
  --help, -h          show this help
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || { echo "error: --version needs a value" >&2; exit 2; }
            VERSION="$2"
            shift 2
            ;;
        --user)
            [ "$#" -ge 2 ] || { echo "error: --user needs a value" >&2; exit 2; }
            USER_NAME="$2"
            shift 2
            ;;
        --yes|-y)
            ASSUME_YES=1
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ "$(id -u)" -ne 0 ]; then
    echo "error: run this with sudo" >&2
    exit 1
fi
command -v apt-get >/dev/null 2>&1 || {
    echo "error: this installer needs a Debian-based system" >&2
    exit 1
}
command -v systemctl >/dev/null 2>&1 || {
    echo "error: this installer needs systemd" >&2
    exit 1
}

if [ -z "$USER_NAME" ]; then
    USER_NAME="${SUDO_USER:-}"
fi
if [ -z "$USER_NAME" ] || [ "$USER_NAME" = "root" ]; then
    cat >&2 <<'EOF'
error: cannot tell which user should own the kiosk.

Run this from your login user:
  sudo bash install.sh

or name the user explicitly:
  sudo bash install.sh --user yourname
EOF
    exit 1
fi
if ! id "$USER_NAME" >/dev/null 2>&1; then
    echo "error: user $USER_NAME does not exist" >&2
    exit 1
fi

ARCH=$(uname -m)
case "$ARCH" in
    aarch64|arm64) ARCH="aarch64" ;;
    x86_64|amd64) ARCH="x86_64" ;;
    *)
        echo "error: unsupported architecture: $ARCH" >&2
        exit 1
        ;;
esac

if [ -n "$VERSION" ]; then
    VERSION="${VERSION#v}"
    case "$VERSION" in
        ''|*[!0-9.]*)
            echo "error: bad version: $VERSION (expected something like 0.27.1)" >&2
            exit 1
            ;;
    esac
    BASE="https://github.com/darknoxx/CRead/releases/download/v$VERSION"
    LABEL="v$VERSION"
else
    BASE="https://github.com/darknoxx/CRead/releases/latest/download"
    LABEL="latest"
fi
ASSET="cread-linux-$ARCH.tar.gz"

# The Pi OS image is expanded to the card on first boot. 8 GB is the minimum;
# a tiny card would run out during the package install or the first sync.
ROOT_TOTAL=$(df -P -k / | awk 'NR == 2 { print $2 }')
ROOT_FREE=$(df -P -k / | awk 'NR == 2 { print $4 }')
ROOT_TOTAL=${ROOT_TOTAL:-0}
ROOT_FREE=${ROOT_FREE:-0}
if [ "$ROOT_TOTAL" -lt $((7 * 1024 * 1024)) ]; then
    echo "warning: the root filesystem is smaller than 8 GB." >&2
    echo "         8 GB is the minimum, 16 GB is recommended." >&2
fi
if [ "$ROOT_FREE" -lt $((2 * 1024 * 1024)) ]; then
    echo "error: less than 2 GB free on /; free some space first." >&2
    exit 1
fi

cat <<EOF
CRead kiosk installer

  release: $LABEL
  arch:    $ARCH
  user:    $USER_NAME
  vault:   /home/$USER_NAME/Documents/CRead
  binary:  /opt/cread/cread

This will:
  - install cage, xwayland, Mesa Vulkan and minisign with apt
  - download and verify the signed CRead release
  - install the kiosk service and start it
EOF

if [ "$ASSUME_YES" -eq 0 ]; then
    if [ -r /dev/tty ]; then
        printf 'Continue? [y/N] '
        read -r answer </dev/tty || answer=""
    else
        echo "error: no terminal for confirmation; rerun with --yes" >&2
        exit 1
    fi
    case "$answer" in
        y|Y|yes|YES) ;;
        *)
            echo "cancelled"
            exit 1
            ;;
    esac
fi

export DEBIAN_FRONTEND=noninteractive
echo "==> apt update"
apt-get update

echo "==> installing packages"
apt-get install -y --no-install-recommends \
    cage ca-certificates curl xwayland minisign \
    mesa-vulkan-drivers libvulkan1

TMP=""
cleanup() {
    if [ -n "$TMP" ]; then
        rm -rf "$TMP"
    fi
}
trap cleanup EXIT
TMP=$(mktemp -d)

echo "==> downloading $LABEL/$ASSET"
curl -fL --retry 3 --connect-timeout 15 -o "$TMP/$ASSET" "$BASE/$ASSET"
curl -fL --retry 3 --connect-timeout 15 -o "$TMP/$ASSET.sha256" "$BASE/$ASSET.sha256"
curl -fL --retry 3 --connect-timeout 15 -o "$TMP/$ASSET.minisig" "$BASE/$ASSET.minisig"

echo "==> checking checksum"
(cd "$TMP" && sha256sum -c "$ASSET.sha256")

# Keep in sync with minisign.pub and PUBLIC_KEYS in crates/update/src/lib.rs.
write_key() {
    printf 'untrusted comment: CRead signing key\n%s\n' "$1" >"$2"
}
write_key 'RWR3LbtsJ8Lo9nXfml6kkaqThFtCkV8cV6AWZQGeI9qlFOEnX5Odba4E' "$TMP/key-current"
write_key 'RWQ0Us3HVbzcwsxnTyDHFbzST5AAnCsqzbsb27FcBEKHmx1uP1LgVqea' "$TMP/key-spare"

echo "==> checking signature"
if minisign -V -p "$TMP/key-current" -m "$TMP/$ASSET" >/dev/null 2>&1; then
    :
elif minisign -V -p "$TMP/key-spare" -m "$TMP/$ASSET" >/dev/null 2>&1; then
    :
else
    echo "error: the release signature does not match a trusted key." >&2
    exit 1
fi

echo "==> extracting"
mkdir "$TMP/dist"
tar -xzf "$TMP/$ASSET" -C "$TMP/dist"
if [ ! -x "$TMP/dist/cread" ] || [ ! -x "$TMP/dist/packaging/install-kiosk.sh" ]; then
    echo "error: the tarball does not look like a CRead release." >&2
    exit 1
fi

echo "==> installing"
CREAD_USER="$USER_NAME" "$TMP/dist/packaging/install-kiosk.sh" "$TMP/dist/cread"

cat <<EOF

CRead is installed for $USER_NAME.

  service: systemctl status cread-kiosk
  logs:    journalctl -u cread-kiosk -b
  open:    SETTINGS -> WEBDAV, then switch sync on
  update:  SETTINGS -> UPDATE, or sudo cread-update

Reboot once to verify that the kiosk starts by itself:

  sudo reboot
EOF
