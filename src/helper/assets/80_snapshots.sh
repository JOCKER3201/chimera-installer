#!/bin/sh
# 80_snapshots — GRUB entries for booting btrfs snapshots read-only
# (CachyOS-style layout: snapshots in the separate @snapshots subvolume).
# Part of chimera-installer; written from scratch, no grub-btrfs/openSUSE code.
#
# Works on VANILLA GRUB: with no default subvolume (set-default), GRUB reads
# the btrfs tree from the top level, so paths like /@snapshots/N/snapshot/...
# are directly reachable. Each entry loads the snapshot's own kernel and
# mounts it as a read-only root (rootflags=subvol=@snapshots/N/snapshot ro).
#
# This is the rescue path for "the system does not come up": you boot a
# snapshot and run `chimera-rollback N` from its console. A regular rollback
# is done straight from the running system, without this menu.
set -e

[ -d /.snapshots ] || exit 0

fstype="$(grub-probe --target=fs / 2>/dev/null || true)"
[ "$fstype" = "btrfs" ] || exit 0
uuid="$(grub-probe --target=fs_uuid / 2>/dev/null || true)"
[ -n "$uuid" ] || exit 0

first=1
for snapdir in /.snapshots/*/; do
    num="$(basename "$snapdir")"
    case "$num" in
        *[!0-9]*) continue ;;
    esac
    # Snapshot no. 1 (FABRIC) is handled by the separate, permanent 12_fabric entry.
    [ "$num" = 1 ] && continue
    snap="${snapdir}snapshot"
    [ -d "$snap" ] || continue

    kernel=""
    for k in "$snap"/boot/vmlinuz-*; do
        [ -e "$k" ] && kernel="$k"
    done
    [ -n "$kernel" ] || continue
    kver="${kernel##*/vmlinuz-}"

    initrd=""
    for cand in "$snap/boot/initrd.img-$kver" "$snap/boot/initramfs-$kver.img"; do
        [ -e "$cand" ] && initrd="$cand" && break
    done
    [ -n "$initrd" ] || continue
    iname="${initrd##*/}"

    date=""
    desc=""
    if [ -f "${snapdir}info.xml" ]; then
        date="$(sed -n 's:.*<date>\(.*\)</date>.*:\1:p' "${snapdir}info.xml" | head -n1)"
        desc="$(sed -n 's:.*<description>\(.*\)</description>.*:\1:p' "${snapdir}info.xml" | head -n1 | tr -d "'")"
    fi

    if [ "$first" = 1 ]; then
        echo "80_snapshots: adding btrfs snapshot boot entries" >&2
        echo "submenu 'Chimera — btrfs snapshots (read-only)' {"
        first=0
    fi

    title="Snapshot $num"
    [ -n "$date" ] && title="$title — $date"
    [ -n "$desc" ] && title="$title — $desc"
    subvol="@snapshots/$num/snapshot"

    echo "  menuentry '$title (kernel $kver)' {"
    echo "    insmod part_gpt"
    echo "    insmod part_msdos"
    echo "    insmod btrfs"
    echo "    search --no-floppy --fs-uuid --set=root $uuid"
    echo "    linux /$subvol/boot/vmlinuz-$kver root=UUID=$uuid ro rootflags=subvol=$subvol ${GRUB_CMDLINE_LINUX} ${GRUB_CMDLINE_LINUX_DEFAULT}"
    echo "    initrd /$subvol/boot/$iname"
    echo "  }"
done

[ "$first" = 1 ] || echo "}"
