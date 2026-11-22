#!/bin/sh
# Wanderlust's engine: Chromium from the system, CRead's policy, and the
# security updates that keep a browser safe to use. Run as root, once; it is
# safe to run again.
#
#   sudo cread-browser-setup
#
# Chromium comes from the distribution on purpose: apt keeps it patched, which
# CRead's own updater could not do for a browser.
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "run this as root" >&2
    exit 1
fi

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
POLICY="$HERE/cread-chromium-policy.json"
if [ ! -f "$POLICY" ]; then
    # Installed copy in /usr/local/sbin: the policy sits next to it.
    POLICY=/usr/local/share/cread/cread-chromium-policy.json
fi
if [ ! -f "$POLICY" ]; then
    echo "error: cread-chromium-policy.json not found" >&2
    exit 1
fi

export DEBIAN_FRONTEND=noninteractive
echo "==> apt update"
apt-get update

# Debian calls it chromium; older Raspberry Pi OS releases chromium-browser.
PACKAGE=chromium
if ! apt-cache show chromium >/dev/null 2>&1; then
    PACKAGE=chromium-browser
fi
echo "==> installing $PACKAGE and unattended-upgrades"
apt-get install -y --no-install-recommends "$PACKAGE" unattended-upgrades

# Managed policy: uBlock Origin Lite installed for everyone, no password
# store (a kiosk has no keyring to lock it with), no camera, microphone,
# location or notifications, nothing sent to Google that is not needed.
echo "==> chromium policy"
for dir in /etc/chromium/policies/managed /etc/chromium-browser/policies/managed; do
    install -d -m 0755 "$dir"
    install -m 0644 "$POLICY" "$dir/cread.json"
done

# Security updates without anyone logging in. Debian's default rules cover
# its security archive; Raspberry Pi OS ships its Chromium from its own.
echo "==> automatic security updates"
cat >/etc/apt/apt.conf.d/52cread-browser <<'EOF'
// Written by cread-browser-setup: keep the browser patched.
Unattended-Upgrade::Origins-Pattern {
        "origin=Debian,codename=${distro_codename}-security,label=Debian-Security";
        "origin=Raspberry Pi Foundation,codename=${distro_codename},label=Raspberry Pi Foundation";
};
EOF
if [ ! -f /etc/apt/apt.conf.d/20auto-upgrades ]; then
    cat >/etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
EOF
fi
systemctl enable --now unattended-upgrades.service >/dev/null 2>&1 || true

echo "==> done: $("$(command -v chromium || command -v chromium-browser)" --version 2>/dev/null || echo "$PACKAGE")"
