#!/bin/sh
# Stop the kiosk and hand tty1 back to a login. Called by
# cread-backtolinux.service, which CRead's `backtolinux` command starts.
set -eu

systemctl stop cread-kiosk.service
systemctl start getty@tty1.service
chvt 1 2>/dev/null || true
