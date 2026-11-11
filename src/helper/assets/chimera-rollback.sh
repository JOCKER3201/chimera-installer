#!/bin/sh
# chimera-rollback — restoring a btrfs snapshot from the COMMAND LINE, with
# no GUI and no graphics driver (works from a tty console, over SSH, and
# also from a booted read-only snapshot or a live USB).
#
# The CachyOS model: / is a FIXED @ subvolume, and a restore means SWAPPING
# the contents of @ — the old @ is moved aside, and a writable copy of the
# chosen snapshot takes its place. Thanks to that, vanilla GRUB (which
# always mounts subvol=@) boots the restored system after a plain reboot,
# with no grub.cfg regeneration and no bootloader patching.
#
# Snapshots live in the separate @snapshots subvolume (mounted at
# /.snapshots), so swapping @ does not touch them.
#
# Written from scratch for chimera-installer (MIT). Uses only snapper and btrfs.
set -eu

TOP=/run/chimera-rollback
SNAPSUBVOL=@snapshots

err() { echo "chimera-rollback: $*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || err "run as root (doas chimera-rollback ...)"

usage() {
    echo "Usage: chimera-rollback <snapshot-number>"
    echo "       chimera-rollback list        # show the available snapshots"
    echo
    echo "Restores the system to the given snapshot (swaps the @ subvolume)."
    echo "When done, restart the computer: reboot"
}

# The btrfs device the current root lives on (without the [subvol] suffix).
root_dev() {
    findmnt -n -v -o SOURCE / | sed 's/\[.*//'
}

do_list() {
    if command -v snapper >/dev/null 2>&1; then
        snapper -c root list 2>/dev/null && return 0
    fi
    # Fallback: a raw listing of the snapshot directories.
    echo "snapshots in ${SNAPSUBVOL}:"
    for d in "$TOP/$SNAPSUBVOL"/*/snapshot; do
        [ -d "$d" ] || continue
        n=$(basename "$(dirname "$d")")
        echo "  $n"
    done
}

dev=$(root_dev)
[ -n "$dev" ] || err "could not determine the root device"

mkdir -p "$TOP"
mounted=0
cleanup() { [ "$mounted" = 1 ] && umount "$TOP" 2>/dev/null || true; }
trap cleanup EXIT

mount -o subvolid=5 "$dev" "$TOP"
mounted=1

case "${1:-}" in
    ""|-h|--help|help) usage; exit 0 ;;
    list|--list) do_list; exit 0 ;;
esac

num="$1"
case "$num" in
    *[!0-9]*) err "the snapshot number must be numeric (see chimera-rollback list)" ;;
esac

src="$TOP/$SNAPSUBVOL/$num/snapshot"
[ -d "$src" ] || err "snapshot no. $num does not exist ($src)"

[ -e "$TOP/@" ] || err "no @ subvolume — unexpected disk layout"

stamp=$(date +%Y%m%d-%H%M%S)
backup="@_rollback_backup_$stamp"

echo "Restoring the system from snapshot no. $num."
echo "The current @ will be kept as $backup."

# The swap: old @ aside, a writable copy of the snapshot in place of @.
mv "$TOP/@" "$TOP/$backup"
if ! btrfs subvolume snapshot "$src" "$TOP/@"; then
    # Failure — bring the previous @ back so the system stays bootable.
    rm -rf "$TOP/@" 2>/dev/null || true
    mv "$TOP/$backup" "$TOP/@"
    err "could not create the new @ — the pre-rollback state was restored"
fi

echo
echo "Done. Restart the computer to boot the restored system:"
echo "    reboot"
echo
echo "Copy of the previous state: $backup (you can delete it later:"
echo "    doas btrfs subvolume delete /.snapshots/../$backup  — after mounting subvolid=5)."
