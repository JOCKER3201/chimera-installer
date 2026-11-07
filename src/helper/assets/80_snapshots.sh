#!/bin/sh
# 80_snapshots — wpisy GRUB do bootowania migawek btrfs w trybie read-only
# (układ w stylu CachyOS: migawki w osobnym subwolumenie @snapshots).
# Część vanilla-installer; napisane od zera, bez kodu grub-btrfs/openSUSE.
#
# Działa na WANILIOWYM GRUB: brak domyślnego subwolumenu (set-default) sprawia,
# że GRUB czyta drzewo btrfs od top-level, więc ścieżki /@snapshots/N/snapshot/...
# są bezpośrednio osiągalne. Każdy wpis ładuje własny kernel migawki i montuje
# ją jako root read-only (rootflags=subvol=@snapshots/N/snapshot ro).
#
# To ścieżka ratunkowa „gdy system nie wstaje": bootujesz migawkę i z jej
# konsoli wykonujesz `vanilla-rollback N`. Zwykły rollback robi się wprost
# z działającego systemu, bez tego menu.
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
        echo "submenu 'Vanilla — migawki btrfs (read-only)' {"
        first=0
    fi

    title="Migawka $num"
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
