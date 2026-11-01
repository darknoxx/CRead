# CRead on a Raspberry Pi

CRead is a kiosk on a Raspberry Pi 4 or 5: the machine boots into cage (a
small Wayland compositor) and CRead fills the screen. There is no desktop and
no terminal in the way.

The vault stays **local**. WebDAV mirrors it to your server when the network is
there; writing and saving work offline. There is no SMB mount.

## What you need

- Raspberry Pi 4 (2 GB is enough, 4 GB is comfortable) or Pi 5.
- Raspberry Pi OS **Lite, 64-bit** on a good SD card (endurance class) or a USB
  SSD. SSD is better for long-running machines.
- A network cable or Wi-Fi, a USB keyboard, and a display.
- A WebDAV server (Nextcloud, ownCloud, nginx-dav, ...) if you want sync.

## Flash and first boot

1. Use Raspberry Pi Imager: **Raspberry Pi OS Lite (64-bit)**.
2. In the imager's settings: hostname, a user (for example `cread`), SSH with a
   key, Wi-Fi, and the German keyboard/locale if you want them.
3. Boot, log in, and update:

```sh
sudo apt update && sudo apt full-upgrade -y
sudo apt install -y cage ca-certificates
```

For a build on the Pi (the first test), also install the development packages:

```sh
sudo apt install -y build-essential pkg-config git \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev wayland-protocols \
  libasound2-dev libudev-dev libssl-dev
```

## First run, without the service

This is the quickest way to find out whether the GPU and the compositor are
happy. Run it from the Pi's own console (not over SSH; cage needs the seat):

```sh
git clone https://github.com/darknoxx/CRead.git
cd CRead
cargo build --release
cage -- ./target/release/cread
```

Try `--backend gl` first, then `--backend vulkan`:

```sh
cage -- ./target/release/cread --backend gl
```

Check the boot screen, typing, the scrollbar, the manual, the German umlauts,
the screensaver, and whether HDMI audio works. `--size 1280x800` is the mode
that fills a 2x raster; at 1920x1080 CRead uses scale 2 and leaves black bars,
because it never scales fractionally.

## Install the kiosk

Download the signed release for the Pi, verify it, and install it:

```sh
curl -LO https://github.com/darknoxx/CRead/releases/latest/download/cread-linux-aarch64.tar.gz
curl -LO https://github.com/darknoxx/CRead/releases/latest/download/cread-linux-aarch64.tar.gz.sha256
curl -LO https://github.com/darknoxx/CRead/releases/latest/download/cread-linux-aarch64.tar.gz.minisig
sha256sum -c cread-linux-aarch64.tar.gz.sha256
# optional, if rsign/minisign is installed:
rsign verify -p minisign.pub cread-linux-aarch64.tar.gz
tar xzf cread-linux-aarch64.tar.gz
sudo ./packaging/install-kiosk.sh ./cread
```

The script creates the `cread` user (unless it exists), installs the binary to
`/opt/cread/cread` (owned by that user), installs the systemd unit and a
polkit rule for `shutdown`, turns the journal volatile and disables the swap
and the tty1 getty.

Start it:

```sh
sudo systemctl start cread-kiosk
sudo systemctl status cread-kiosk
journalctl -u cread-kiosk -f
```

The unit starts cage on tty1, with `CREAD_VAULT` in the user's home. CRead
starts on its boot screen; press `Esc`, then `open` or `new`.

## WebDAV and offline

Open `SETTINGS -> WEBDAV`, fill in server, user, password and folder, and
switch `sync` on. The `sync.json` is written with mode `0600`.

CRead is local-first:

- Every save goes to the local vault first. Offline writing works.
- Sync runs at startup, after a save, every 60 seconds, and on `sync`.
- If both the local and the server copy changed, the local file stays and the
  server copy is parked next to it as `name.conflict-<unix>.<ext>`.
- Documents are never deleted on the server.

Use one writer per file. Two machines editing the same file at the same time
produce a conflict copy, not a merge. That is the same policy the desktop
build uses.

The WebDAV password can also come from `CREAD_WEBDAV_PASSWORD` in the service
environment if you do not want it in the config file; `Config::password` checks
that variable first.

## Updates

`SETTINGS -> UPDATE` works from inside CRead. The binary is user-owned, so the
signed updater can replace it without root. With `--kiosk` it does **not**
spawn a new process: it saves, exits, and systemd starts cage + CRead again.
You see the boot bar, not a terminal.

If the kiosk is too broken to reach SETTINGS, the headless fallback is:

```sh
sudo cread-update
```

It stops the service, runs `cread --update-once` as the kiosk user (checksum
and minisign are verified), and starts the service again.

Do not put `/opt/cread` on an overlay or read-only root: the in-app update
would disappear on the next reboot. If you want a read-only root later, move
`/opt/cread` and the home directory to a persistent partition.

## SD cards and long life

A kiosk writes less than a desktop, but a few things are worth doing:

- Use an endurance card (Samsung Pro Endurance, SanDisk Max Endurance) or boot
  from USB SSD.
- Leave autosave off (the default). CRead writes on `save`, `saveas`, quitting
  and closing the window.
- The sync index is only written when it changed. A quiet round costs no SD
  write.
- The journal is volatile and swap is off (the install script does both).
- Add `noatime` to the root entry in `/etc/fstab`.
- Keep the boot quiet: `quiet` in `/boot/firmware/cmdline.txt`,
  `disable_splash=1` in `/boot/firmware/config.txt`, and the console on tty3.

## Shutdown

`shutdown` (alias `poweroff`) is a command in the prompt. It asks once, saves
the document and the running program locally, and calls the system's power-off.
WebDAV catches up on the next start. The polkit rule installed with the kiosk
allows the `cread` user to do that without a password. `exit!` only leaves the
session; systemd starts it again.

## Known limits

- No SMB. The vault is local + WebDAV on purpose; see above.
- One writer per file; conflicts are copies, not merges.
- 32-bit Raspberry Pi OS is not supported.
- On a Pi 4, Vulkan depends on the Mesa `v3dv` driver. If it is unhappy, use
  `--backend gl`.
