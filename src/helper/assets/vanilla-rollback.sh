#!/bin/sh
# vanilla-rollback — przywracanie migawki btrfs z WIERSZA POLECEŃ, bez GUI
# i bez sterownika graficznego (działa z konsoli tty, przez SSH, a także
# z bootniętej migawki read-only albo z live USB).
#
# Model jak w CachyOS: / to STAŁY subwolumen @, a przywrócenie polega na
# PODMIANIE zawartości @ — stare @ idzie na bok, w jego miejsce powstaje
# zapisywalna kopia wybranej migawki. Dzięki temu waniliowy GRUB (który
# zawsze montuje subvol=@) po zwykłym reboocie wstaje z przywróconego
# systemu, bez żadnej regeneracji grub.cfg ani łatania bootloadera.
#
# Migawki leżą w osobnym subwolumenie @snapshots (montowanym na /.snapshots),
# więc podmiana @ ich nie narusza.
#
# Napisane od zera dla vanilla-installer (MIT). Używa tylko snappera i btrfs.
set -eu

TOP=/run/vanilla-rollback
SNAPSUBVOL=@snapshots

err() { echo "vanilla-rollback: $*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || err "uruchom jako root (doas vanilla-rollback ...)"

usage() {
    echo "Użycie: vanilla-rollback <numer-migawki>"
    echo "        vanilla-rollback list        # pokaż dostępne migawki"
    echo
    echo "Przywraca system do wskazanej migawki (podmiana subwolumenu @)."
    echo "Po wykonaniu zrestartuj komputer: reboot"
}

# Urządzenie btrfs, na którym leży bieżący root (bez sufiksu [subvol]).
root_dev() {
    findmnt -n -v -o SOURCE / | sed 's/\[.*//'
}

do_list() {
    if command -v snapper >/dev/null 2>&1; then
        snapper -c root list 2>/dev/null && return 0
    fi
    # Zapas: surowy spis katalogów migawek.
    echo "migawki w ${SNAPSUBVOL}:"
    for d in "$TOP/$SNAPSUBVOL"/*/snapshot; do
        [ -d "$d" ] || continue
        n=$(basename "$(dirname "$d")")
        echo "  $n"
    done
}

dev=$(root_dev)
[ -n "$dev" ] || err "nie udało się ustalić urządzenia roota"

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
    *[!0-9]*) err "numer migawki musi być liczbą (zob. vanilla-rollback list)" ;;
esac

src="$TOP/$SNAPSUBVOL/$num/snapshot"
[ -d "$src" ] || err "migawka nr $num nie istnieje ($src)"

[ -e "$TOP/@" ] || err "brak subwolumenu @ — nieoczekiwany układ dysku"

stamp=$(date +%Y%m%d-%H%M%S)
backup="@_rollback_backup_$stamp"

echo "Przywracam system z migawki nr $num."
echo "Bieżący @ zostanie zachowany jako $backup."

# Podmiana: stare @ na bok, zapisywalna kopia migawki w miejsce @.
mv "$TOP/@" "$TOP/$backup"
if ! btrfs subvolume snapshot "$src" "$TOP/@"; then
    # Awaria — przywróć poprzedni @, żeby system pozostał rozruchowy.
    rm -rf "$TOP/@" 2>/dev/null || true
    mv "$TOP/$backup" "$TOP/@"
    err "nie udało się utworzyć nowego @ — przywrócono stan sprzed rollbacku"
fi

echo
echo "Gotowe. Zrestartuj komputer, aby uruchomić przywrócony system:"
echo "    reboot"
echo
echo "Kopia poprzedniego stanu: $backup (możesz ją później usunąć:"
echo "    doas btrfs subvolume delete /.snapshots/../$backup  — po zamontowaniu subvolid=5)."
