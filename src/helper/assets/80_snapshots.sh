#!/bin/sh
# 80_snapshots — wpisy GRUB dla migawek btrfs w układzie w stylu openSUSE
# (root w @/.snapshots/N/snapshot). Część vanilla-installer.
#
# Skrypt napisany od zera na potrzeby tego instalatora; NIE zawiera kodu
# z grub2-snapper-plugin ani innych skryptów openSUSE — odtwarza jedynie
# zachowanie: podmenu z możliwością rozruchu każdej migawki.
#
# Wpisy montują migawkę przez rootflags=subvol=..., w trybie ro.
set -e

# Niełatany grub-mkconfig nie eksportuje obcych zmiennych do skryptów
# w grub.d — flagę doczytujemy wprost z /etc/default/grub.
if [ -z "${SUSE_BTRFS_SNAPSHOT_BOOTING}" ] && [ -f /etc/default/grub ]; then
    . /etc/default/grub
fi
# Zgodnie z konwencją openSUSE: generacja tylko przy włączonej fladze.
if [ "x${SUSE_BTRFS_SNAPSHOT_BOOTING}" != "xtrue" ]; then
    exit 0
fi
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
        echo "80_snapshots: dodaję wpisy rozruchowe migawek btrfs" >&2
        echo "submenu 'Chimera Linux — migawki btrfs' {"
        first=0
    fi

    title="Migawka $num"
    [ -n "$date" ] && title="$title — $date"
    [ -n "$desc" ] && title="$title — $desc"

    echo "  menuentry '$title (kernel $kver)' {"
    echo "    insmod part_gpt"
    echo "    insmod part_msdos"
    echo "    insmod btrfs"
    echo "    search --no-floppy --fs-uuid --set=root $uuid"
    echo "    linux /@/.snapshots/$num/snapshot/boot/vmlinuz-$kver root=UUID=$uuid ro rootflags=subvol=@/.snapshots/$num/snapshot ${GRUB_CMDLINE_LINUX} ${GRUB_CMDLINE_LINUX_DEFAULT}"
    echo "    initrd /@/.snapshots/$num/snapshot/boot/$iname"
    echo "  }"
done

[ "$first" = 1 ] || echo "}"
