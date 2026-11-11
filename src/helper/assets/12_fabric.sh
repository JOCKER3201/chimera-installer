#!/bin/sh
# 12_fabric — a permanent, dedicated GRUB entry that boots the protected
# FABRIC factory snapshot (no. 1). The snapshot is writable, so it comes up
# as a full system (dinit remounts / as rw — just like on a regular boot);
# the `ro` below is the standard boot mode, not a lock. The script lives in
# /etc/grub.d, i.e. inside the @ subvolume, so it is present in EVERY
# snapshot — thanks to that, after any rollback (which restores grub.cfg
# together with @) the FABRIC entry is still generated.
# Snapshot no. 1 is undeletable (chattr +i on the directory, empty cleanup),
# so this entry is always current. Part of chimera-installer, written from
# scratch.
set -e

snap=/.snapshots/1/snapshot
[ -d "$snap" ] || exit 0

fstype="$(grub-probe --target=fs / 2>/dev/null || true)"
[ "$fstype" = "btrfs" ] || exit 0
uuid="$(grub-probe --target=fs_uuid / 2>/dev/null || true)"
[ -n "$uuid" ] || exit 0

kernel=""
for k in "$snap"/boot/vmlinuz-*; do
    [ -e "$k" ] && kernel="$k"
done
[ -n "$kernel" ] || exit 0
kver="${kernel##*/vmlinuz-}"

initrd=""
for cand in "$snap/boot/initrd.img-$kver" "$snap/boot/initramfs-$kver.img"; do
    [ -e "$cand" ] && initrd="$cand" && break
done
[ -n "$initrd" ] || exit 0
iname="${initrd##*/}"
subvol="@snapshots/1/snapshot"

echo "12_fabric: adding the permanent FABRIC factory snapshot entry" >&2
cat <<EOF
menuentry 'Restore factory settings' {
    insmod part_gpt
    insmod part_msdos
    insmod btrfs
    search --no-floppy --fs-uuid --set=root $uuid
    linux /$subvol/boot/vmlinuz-$kver root=UUID=$uuid ro rootflags=subvol=$subvol ${GRUB_CMDLINE_LINUX} ${GRUB_CMDLINE_LINUX_DEFAULT}
    initrd /$subvol/boot/$iname
}
EOF
