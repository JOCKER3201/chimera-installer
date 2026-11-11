# chimera-installer

Graphical installer for **Chimera Linux** written in Rust (GTK4 + libadwaita),
based on the official procedure from <https://chimera-linux.org/docs/installation>.

> **Authorship:** this project was written entirely by an AI — **Claude
> (Anthropic)** — commissioned by and to the specification of the user.

## What it does

* **Two installation modes** (chosen right after startup):
  * **from a Chimera live medium** — with a package source choice:
    * *local* (`chimera-bootstrap -f -l`) — a faithful copy of the live
      system (including the medium's desktop), kernel inherited from the
      medium; services enabled on the live system (`/etc/dinit.d/boot.d`)
      are mirrored in the target system; snapper and CPU microcode are
      fetched from the network,
    * *network* (`chimera-bootstrap -f`) — the latest packages from the
      repository, plus a system choice: **base system / GNOME / KDE Plasma**,
      and for desktops additionally a **base** edition (no applications,
      terminal only: `apk add gnome '!gnome-apps'` + `gnome-console`, or
      `plasma-desktop` without `*-apps-meta`/`*-multimedia-meta` + `konsole`)
      or **full** (the complete metapackage: `apk add gnome` /
      `apk add plasma-desktop`),
  * **from another system** — downloads the latest
    `chimera-linux-<arch>-ROOTFS-<date>-bootstrap.tar.gz` image from
    `repo.chimera-linux.org` (with SHA-256 verification), unpacks it to
    `/tmp/chimera-linux`, and then runs `chimera-bootstrap` inside it via
    **chroot** against the target disk; always behaves like a network
    installation (with the base / GNOME / KDE choice).
* **System ready after the first boot** — dinit services are enabled in the
  chroot with symlinks in `/etc/dinit.d/boot.d/` (exactly what
  `dinitctl enable` does; it cannot run in a chroot — it needs a live
  daemon): `syslog-ng`, the network daemon (`dhcpcd` for base,
  `networkmanager` for desktops), the login manager (`gdm`/`sddm`) and
  `snapper`. Services enabled by default through the `*-dinit-links`
  packages (dbus, elogind, chrony, turnstiled, udevd, pipewire/wireplumber
  per-user) are not duplicated by the installer. The network installation
  explicitly adds `dinit-chimera-udev` (base-full ships the `device-none`
  stub without udevd) and CPU microcode (`ucode-amd`/`ucode-intel`
  depending on the processor).
* **Simple partitioning** — you only pick a disk:
  * empty disk → installation on the whole disk (GPT; a 1 GiB ESP on UEFI,
    a 1 MiB "BIOS boot" partition on BIOS),
  * disk with free space → asks whether to use the free space (existing
    partitions stay untouched; an existing ESP is reused),
  * no free space → asks about switching to **advanced mode** (for now a
    **stub** — the button only informs), with an option to wipe the whole
    disk as well.
* **btrfs tailored for snapper in the CachyOS layout** (a deliberate choice
  over openSUSE's — rollback then works on **vanilla GRUB**, with no
  bootloader patching and straight from the console):
  * flat top-level subvolumes: `@` (root), `@home`, `@root`, `@srv`,
    `@cache` (/var/cache), `@tmp` (/var/tmp), `@log` (/var/log, with
    `chattr +C` like `@cache`), and `@snapshots` on `/.snapshots`,
  * `/` is mounted as a **fixed `subvol=@`** — this is what makes a rollback
    (swapping the contents of `@`) visible to unpatched GRUB,
  * `/boot` stays **inside `@`**, so a rollback also covers the
    kernel/initrd (better than CachyOS with Limine, where the kernel lives
    on the ESP outside the snapshots),
  * `@snapshots` is a **separate** subvolume (outside `@`), so snapshots
    survive the swap of `@` without any special handling,
  * `/etc/snapper/configs/root` (`NUMBER_LIMIT="2-10"`,
    `TIMELINE_CREATE="no"`, …), the `SNAPPER_CONFIGS="root"` registry, the
    `snapper` (dinit) service enabled; an initial "system installation"
    snapshot is created,
  * the `/etc/grub.d/80_snapshots` generator adds a submenu with
    **read-only** snapshot boot entries (paths from the top level,
    `/@snapshots/N/snapshot/…`, works on vanilla GRUB) — a rescue path for
    when the system does not come up,
  * the `chimera-grub-watch` dinit service (via `inotify-tools`) watches
    `/.snapshots` and **automatically runs `update-grub` after every
    snapshot creation or deletion** (the equivalent of `grub-btrfs.path`
    from CachyOS), so the snapshot submenu is always up to date,
  * the system is **installed directly into the factory snapshot "FABRIC"**
    (`/.snapshots/1/snapshot`), and at the end of the installation the
    permanent root subvolume is created from it:
    `btrfs subvolume snapshot FABRIC → @` (GRUB is installed only in `@`,
    so the prefix in the ESP points at `@`, not at the snapshot). FABRIC
    remains **writable** — when booted it comes up as a full system,
    desktop included (a read-only snapshot ended up in the console: with no
    writes, seedrng, machine-id and the login manager all failed). The
    empty `cleanup` field in its metadata keeps snapper's automatic cleanup
    away from it, and the snapshot directory gets `chattr +i`, which blocks
    `snapper delete`/`btrfs subvolume delete` until someone deliberately
    removes the immutable flag (`doas chattr -i /.snapshots/1`) — a
    deterrent, not an absolute lock (root can always undo it). GRUB's
    **main menu** gains a **"Restore factory settings"** entry (the
    `/etc/grub.d/12_fabric` generator) which boots FABRIC as a full
    system (to permanently restore the factory state onto `@`:
    `doas chimera-rollback 1`; a fresh `/boot/grub` is copied into FABRIC,
    so GRUB has its config after such a rollback). Note: writes made during
    a FABRIC session stay in it permanently. The generator lives in `@`,
    so it is present in every snapshot — after any rollback that restores
    `grub.cfg` together with `@`, the entry is still generated.
* **Command-line rollback** — the `chimera-rollback` tool (installed into
  `/usr/bin`) restores a snapshot by swapping `@` (like CachyOS/Timeshift),
  **with no GUI and no graphics driver**: `doas chimera-rollback list`,
  `doas chimera-rollback <N>`, `reboot`. It works from a `tty` console,
  over SSH, from a booted read-only snapshot, and even from a live USB.
  `snapper rollback` (openSUSE, based on `set-default`) is deliberately NOT
  used — vanilla GRUB would ignore it.
* **"System" and "User" pages** — computer name, time zone
  (continent → zone dialog), language (`LANG` in `/etc/locale.conf`, also
  effective in the tty), keyboard layout (layout + variant dialog; written
  to `/etc/default/keyboard` for the console and
  `/etc/X11/xorg.conf.d/00-keyboard.conf` for the graphical session) and an
  **SSH server** switch (sshd — disabled by default, even when the live
  medium had it enabled); the user and root accounts are on a separate page.
* **pkexec instead of sudo** — the GUI runs unprivileged; all the work is
  done by a helper (`<binary> helper`) launched through `pkexec`, and the
  configuration (passwords included) travels exclusively over stdin.

## Building

Required: Rust (stable), `gtk4-devel`, `libadwaita-devel` (≥ 1.5).

```sh
cargo build --release
# binary: target/release/chimera-installer
```

On immutable systems (Bazzite/Silverblue) the most convenient way is to
build inside a distrobox with the devel packages installed.

## Usage

```sh
./chimera-installer
```

The installer asks for polkit authorization only at the moment the
installation starts. In the target system a user is created in the `wheel`
group (administration via `doas`, matching Chimera's default
configuration); the root account can be locked by leaving its password
empty. For convenience a `/usr/bin/sudo` → `doas` symlink is created, so
`sudo …` commands work too.

Installed packages: `base-full` (via `chimera-bootstrap`), `linux-stable`
(except in local mode — there the kernel comes with the live copy),
`btrfs-progs`, `snapper` (the `user` repository is added automatically),
`openssl`, `dinit-chimera-udev`, CPU microcode,
`grub-x86_64-efi`+`efibootmgr` (UEFI) or `grub-i386-pc` (BIOS); for
desktops additionally `gnome`+`gdm` or `plasma-desktop`+`sddm` plus
`networkmanager`.

## Experimental: proprietary NVIDIA driver

For NVIDIA cards, **in live mode only**, the graphics card screen
additionally offers the proprietary drivers (branch **620** — Turing and
newer, or **580** — Maxwell/Pascal). Since the packages are not in the
Chimera repository yet, the installer **builds them on the live medium**
from the open cports PR (#6531) using `cbuild`:

1. installs the build dependencies (`base-cbuild-host`, `ruff`),
2. clones cports into `/tmp/musl-nvidia` (the current tip of the PR
   branch; the PR gets force-pushed, so a fixed commit pin quickly becomes
   unreachable — the SHA that was used is logged),
3. builds the packages as a regular user (cbuild refuses to run as root),
4. installs the resulting `.apk` files into the target system (`apk --root
   --allow-untrusted`), together with `linux-stable-devel` and `*-ckms`;
   the kernel module is built by the ckms trigger, and `musl` is swapped
   for `musl-nvidia` in the same transaction; the nouveau blacklist and
   the GSP firmware are provided by the packages themselves.

The option is **experimental**: the PR is marked as a draft, the build
downloads ~1.5–2 GB and takes several GB of disk space (beware of `tmpfs`
on `/tmp` on a live medium), and due to a known apk bug the kernel module
may only get built on the first boot. The installer shows a warning before
the option can be selected.

## Restoring snapshots (rollback)

Rollback works **entirely from the command line**, with no GUI and no
graphics driver, on unpatched GRUB — because `/` is a fixed `subvol=@` and
a restore swaps the *contents* of `@` (the CachyOS/Timeshift model):

1. from the running system or from a `tty` console (when only the desktop
   died), over SSH, from a booted read-only snapshot or from a live USB:
   ```sh
   doas chimera-rollback list      # numbers of the available snapshots
   doas chimera-rollback 7         # restore snapshot no. 7
   reboot
   ```
2. after the reboot GRUB mounts the fixed `subvol=@`, which already
   contains the restored contents — **no `update-grub`, no menu choice**.

The previous state is kept as `@_rollback_backup_<date>` (it can be
deleted later). openSUSE's `snapper rollback` (based on `set-default`) is
deliberately NOT used, because vanilla GRUB would ignore it.

Manually from a live USB (when not even the console comes up) — the same
thing `chimera-rollback` does:
```sh
doas mount -o subvolid=5 /dev/<btrfs-partition> /mnt
doas mv /mnt/@ /mnt/@_backup
doas btrfs subvolume snapshot /mnt/@snapshots/<N>/snapshot /mnt/@
doas umount /mnt && reboot
```

## Limitations of the current version

* The advanced partitioning mode is a stub (by design).
* Target architecture: the one the installer runs on (tested assumptions:
  x86_64; BIOS/UEFI mode detected from the host).
* No swap partition is created.
* The time zone and keyboard layout have no dedicated pages — the zone is
  a text field (detected from the host by default).

## License and configuration provenance

Code: MIT license (see `LICENSE`).

The snapper/GRUB configuration **mirrors the values and layout** known
from openSUSE (github.com/openSUSE); these are configuration data, not
code. The `80_snapshots` script was written from scratch for this project
and **contains no code** from `grub2-snapper-plugin` or other GPL sources
— it only reproduces the behavior. Chimera's tools (`chimera-bootstrap`,
`chimera-chroot`, `apk`, `update-grub`) are merely **invoked**.
