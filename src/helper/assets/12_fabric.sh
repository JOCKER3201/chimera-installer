#!/bin/sh
# 12_fabric — stały, wyróżniony wpis GRUB uruchamiający chronioną migawkę
# fabryczną FABRIC (nr 1) w trybie read-only. Leży w /etc/grub.d, czyli
# w subwolumenie @, więc jest w KAŻDEJ migawce — dzięki temu po dowolnym
# rollbacku (który przywraca grub.cfg razem z @) wpis FABRIC nadal powstaje.
# Migawka nr 1 jest niemodyfikowalna (chattr +i) i nieusuwana (puste cleanup),
# więc ten wpis jest zawsze aktualny. Część chimera-installer, napisane od zera.
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

echo "12_fabric: dodaję stały wpis migawki fabrycznej FABRIC" >&2
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
