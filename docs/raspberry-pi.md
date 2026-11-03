# CRead OS on a Raspberry Pi

This is the supported way to turn a Raspberry Pi 4 or 5 into a CRead kiosk:
Raspberry Pi OS Lite, the cage Wayland compositor, CRead in fullscreen, and a
signed release you can update from inside CRead. This dokument will guide you through the installation.

The vault can be found **on the Pi**. WebDAV mirrors it to your server when the
network is configured; writing and saving work offline.

## What you need

- Raspberry Pi 4 (2 GB is enough, 4 GB is comfortable) or Pi 5.
- A micro-HDMI cable, a display, a USB keyboard, and a network cable or Wi-Fi.
- A microSD card: **8 GB minimum**. Use a larger SD card if you sync
  a lot of pictures or ROMs, or use an endurance card.
- Optional: a WebDAV server (Nextcloud, ownCloud, nginx-dav, ...) if you want
  sync.

## Quick install (recommended)

If you just want the kiosk, flash the card as described in section 1, log in
over SSH, and run the signed installer:

```sh
curl -fLO https://github.com/darknoxx/CRead/releases/latest/download/install.sh
less install.sh
sudo bash install.sh
```

The script asks for confirmation, installs cage, xwayland, Mesa Vulkan and
minisign, downloads the release, checks its SHA256 and minisign signature
against the keys baked into the script, and runs the same kiosk installer
described below. It uses the user that invoked `sudo`; pass `--user NAME` if
that is not what you want, `--version 0.31.2` to pin a release, or `--yes` for
an unattended run. The examples use `creaduser` and `creados`, but any username
and hostname work; the installer reads the user from `sudo`, not from a fixed
name.

Reboot once afterwards, then open `SETTINGS → WEBDAV`. The numbered sections
below are the same steps by hand, plus recovery and troubleshooting; you do not
need both.

## 1. Flash Raspberry Pi OS Lite (64-bit)

Do not use the Desktop image; the kiosk is the only program on the machine.

1. Install [Raspberry Pi Imager](https://www.raspberrypi.com/software/) on your
   computer.
2. Choose **Raspberry Pi OS (other) → Raspberry Pi OS Lite (64-bit)**.
3. Choose the SD card.
4. Open the imager's settings (the gear) and set:
   - **Hostname:** for example `creados`
   - **Username and password:** this becomes your kiosk and maintenance user.
     There is no default password in CRead; set a real one.
   - **SSH:** public-key authentication is best; import your
     `~/.ssh/id_ed25519.pub`. Password SSH is okay on a trusted network.
   - **Wi-Fi:** your network, if you do not use Ethernet.
   - **Locale, keyboard and time zone:** for example German, `de`, your zone.
5. Write the card and boot the Pi.

If you flash without these settings, the Pi may have no usable user. Flash it
again with the imager settings; that is the supported path for this manual
installation.

## 2. First boot and system packages

Log in on the Pi (console or SSH) as the user you created.

```sh
uname -m
```

This must print `aarch64`. Then update the system:

```sh
sudo apt update
sudo apt full-upgrade -y
```

Reboot if the kernel or firmware was updated:

```sh
sudo reboot
```

After logging in again, install cage and the pieces CRead needs:

```sh
sudo apt install -y cage ca-certificates curl xwayland
```

`cage` is the kiosk compositor; `xwayland` is required by cage even though
CRead itself is a Wayland client.

Optional: the Vulkan backend (CRead also works with OpenGL ES on the Pi):

```sh
sudo apt install -y mesa-vulkan-drivers libvulkan1 vulkan-tools
vulkaninfo --summary
```

## 3. Download CRead

Use the latest signed release for the Pi. Run these lines in **one shell**, so
`base` is still set:

```sh
cd ~
base=https://github.com/darknoxx/CRead/releases/latest/download

curl -fLO "$base/cread-linux-aarch64.tar.gz"
curl -fLO "$base/cread-linux-aarch64.tar.gz.sha256"
curl -fLO "$base/cread-linux-aarch64.tar.gz.minisig"

sha256sum -c cread-linux-aarch64.tar.gz.sha256
```

The checksum must say `OK`. `curl -f` refuses to save an HTML error page as a
release file.

The release is also signed with minisign. The public key is in the repository:

```sh
curl -fLO https://raw.githubusercontent.com/darknoxx/CRead/main/minisign.pub
sudo apt install -y minisign
minisign -V -p minisign.pub -m cread-linux-aarch64.tar.gz
```

Extract it:

```sh
tar xzf cread-linux-aarch64.tar.gz
```

You now have `cread`, `packaging/`, `RASPBERRY-PI.md` and the licenses in your
home directory.

## 4. First run on the console

Run this **on the Pi's own display and keyboard**, not over SSH: cage needs the
local seat.

```sh
cage -- ./cread
```

CRead starts on its boot screen: the logo, the drives (`A: INSPIRO`,
`B: SOFTHAND`, ...) and a prompt. If the graphics driver is unhappy, try:

```sh
cage -- ./cread --backend gl
```

The libseat lines about `seatd` are normal: libseat tries seatd and falls back
to logind. If cage exits instead, see Troubleshooting.

Optional: configure WebDAV now with `settings → WEBDAV` and `sync` on. The
configuration is stored in `~/.config/cread/sync.json` and survives the kiosk
installation as long as you install with the same user.

To leave CRead, press `Esc`, type `exit!` and press `Enter`. You are back in
the shell and cage has ended.

## 5. Install the kiosk

From the directory where you extracted the release:

```sh
sudo CREAD_USER=$USER ./packaging/install-kiosk.sh ./cread
```

The script:

- installs the binary to `/opt/cread/cread`, owned by your user, so
  `SETTINGS → UPDATE` can replace it without root;
- installs `/etc/systemd/system/cread-kiosk.service` and a polkit rule that
  lets `shutdown` work without a password;
- turns the journal volatile and disables the swap unit for the next boot;
- disables `getty@tty1`, installs `/usr/local/sbin/cread-update`, and
  starts/restarts the kiosk.

If you leave out `CREAD_USER`, the script creates a separate `cread` user and
uses `/home/cread` instead. For a single-user Pi, `CREAD_USER=$USER` is the
simpler choice.

Reboot to test the real kiosk path:

```sh
sudo reboot
```

The Pi should boot straight into CRead's boot screen, with no login prompt.
Check it from SSH:

```sh
systemctl status cread-kiosk --no-pager
journalctl -u cread-kiosk -b --no-pager | tail -40
```

If the screen stays on a bare cursor or the Linux console, see Troubleshooting.

## 6. WebDAV, storage and the kiosk commands

Open `SETTINGS → WEBDAV`, fill in server, user, password and folder, and switch
`sync` on. The `sync.json` is written with mode `0600`.

CRead is local-first:

- every save goes to the local vault first, so offline writing works;
- sync runs at startup, after a save, every 60 seconds, and on `sync`;
- if both sides changed, the local file stays and the server copy is kept as
  `name.conflict-<unix>.<ext>`;
- documents are never deleted on the server.

Use one writer per file. Two machines editing the same file at the same time
produce a conflict copy, not a merge.

`SETTINGS → STORAGE` shows a ten-cell bar, the percentage and used/total space
of the vault's filesystem. `Enter` refreshes it. A later USB SSD shows its own
filesystem.

Commands worth knowing:

- `exit!` saves and ends the session; systemd starts cage and CRead again.
- `shutdown` (alias `poweroff`) asks once, saves and powers the machine off.
- `settings`, `open`, `new`, `save`, `saveas` work as on the desktop.

The kiosk hides the OS pointer until the mouse is moved or clicked. A Pi
without a mouse never shows an arrow.

## 7. Updates

Open `SETTINGS → UPDATE` and confirm. CRead downloads the latest signed release,
checks the checksum and the minisign signature, replaces `/opt/cread/cread`,
keeps the old binary as `cread.old`, and exits. Because the service runs CRead
with `--kiosk`, the process does not spawn a second copy: systemd starts cage
and the new binary together. You see the boot bar again; no terminal.

Headless fallback, for a kiosk that will not come up far enough:

```sh
sudo cread-update
```

It stops the service, runs `cread --update-once` as the kiosk user (the same
checksum and signature checks), and starts the service again.

If a new binary does not start, roll back from SSH:

```sh
sudo systemctl stop cread-kiosk
sudo cp /opt/cread/cread.old /opt/cread/cread
sudo systemctl start cread-kiosk
```

The unit and install script are not updated by the binary updater. Re-run the
installation from a newer tarball if you want those changes too.

## 8. Updating Raspberry Pi OS

CRead OS updates itself through `SETTINGS → UPDATE`. The operating system
(cage, Mesa, the kernel) is updated from a Linux console:

```
backtolinux
```

CRead asks once, saves, stops the kiosk and opens a login on the screen. Log in
as your kiosk user, then:

```sh
sudo apt update
sudo apt full-upgrade -y
sudo reboot
```

The kiosk service is enabled, so CRead starts again after the reboot. If you
prefer not to reboot, return with:

```sh
sudo systemctl start cread-kiosk
```

`reboot` inside CRead restarts the machine; `shutdown` powers it off. Do not
use an overlay or read-only root: the updater and the system updates need to
write to the card.

## 9. Maintenance and recovery

SSH is the maintenance door. The kiosk user is the user from the imager.

```sh
systemctl status cread-kiosk --no-pager
journalctl -u cread-kiosk -b -f
```

Stop the kiosk and get the console back:

```sh
sudo systemctl stop cread-kiosk
sudo systemctl start getty@tty1
```

Start it again:

```sh
sudo systemctl start cread-kiosk
```

If the display stays black but SSH works, the Pi may not see the monitor at
all. Check the connector:

```sh
for f in /sys/class/drm/card*-HDMI-A-*/status; do echo "$f: $(cat "$f")"; done
```

`connected` is what you want. If both say `disconnected`, check the monitor
power, the HDMI input, and the cable, and try the other micro-HDMI port (the
Pi 4 has two). The kiosk keeps running headless, so reconnecting the cable and
restarting the service is enough.

Force a backend if the default one is unhappy:

```sh
sudo systemctl edit --full cread-kiosk
# change ExecStart to:
# /usr/bin/cage -- /opt/cread/cread --kiosk --backend gl
sudo systemctl daemon-reload
sudo systemctl restart cread-kiosk
```

## 10. The SD card

A kiosk writes little, but a few settings are worth having:

- 8 GB is the minimum; 16 GB is comfortable; 32 GB if you sync ROMs or many
  pictures. Endurance cards are worth the extra money.
- Autosave stays off (the default). CRead writes on `save`, `saveas`, quitting
  and closing the window; the sync index is only written when it changed.
- Add `noatime` to the root entry in `/etc/fstab`, then reboot.
- The installer already turns the journal volatile and disables swap for the
  next boot.
- Optional quiet boot: add `quiet` to `/boot/firmware/cmdline.txt` and
  `disable_splash=1` to `/boot/firmware/config.txt`.
- Do **not** use an overlay or read-only root: the in-app updater writes
  `/opt/cread`, and those changes would vanish on the next reboot. If you want
  a read-only root later, move `/opt/cread` and the home directory to a
  persistent partition.

## 11. Troubleshooting

**`sha256sum` says no properly formatted checksum line.** You downloaded an
HTML error page or the wrong file. Re-run the download with `curl -fLO` and the
pinned release URL:

```sh
base=https://github.com/darknoxx/CRead/releases/download/v0.31.2
curl -fLO "$base/cread-linux-aarch64.tar.gz.sha256"
```

**cage prints `seatd` warnings and exits.** The two libseat lines are normal
when seatd is absent; libseat falls back to logind. Run cage on the Pi's own
console, not over SSH, and not with `sudo`. Check the session:

```sh
tty
loginctl
```

The session must be on `seat0` and `tty1`. Over SSH there is no seat.

**`Cannot create XWayland server`.** Install `xwayland` (step 2). CRead itself
is a Wayland client, but cage starts Xwayland for X11 clients.

**`limit 'max_color_attachments' value 8 is better than allowed 4`.** This was
fixed in CRead 0.26.1. Update to a current release.

**Black screen with no signal, but SSH works.** The Pi sees no monitor. Check
the cable and the monitor's input, use the other HDMI port, and look at the
`/sys/class/drm/.../status` values above.

**The installer ends with a bare cursor.** The old installer only enabled the
service. Current releases start it; if you are on an old tarball:

```sh
sudo systemctl start cread-kiosk
```

**The in-app update fails.** Use `sudo cread-update`; if that fails too, check
the network and the journal, then install the next release from a tarball.
