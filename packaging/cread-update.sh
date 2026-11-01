#!/bin/sh
# Headless fallback for a kiosk that will not come up far enough to use
# SETTINGS -> UPDATE. Run as root; the binary stays user-owned.
set -eu

USER_NAME=${CREAD_USER:-cread}

systemctl stop cread-kiosk.service 2>/dev/null || true
status=0
runuser -u "$USER_NAME" -- /opt/cread/cread --update-once || status=$?
systemctl start cread-kiosk.service
exit "$status"
