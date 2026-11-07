#!/bin/sh
# vanilla-grub-watch — pilnuje katalogu migawek i po każdym utworzeniu lub
# usunięciu migawki automatycznie odświeża menu GRUB (update-grub), żeby
# podmenu „migawki btrfs" było zawsze aktualne. Odpowiednik grub-btrfs.path
# z CachyOS, przystosowany do dinit. Napisane od zera dla vanilla-installer.
set -u

DIR=/.snapshots

command -v inotifywait >/dev/null 2>&1 || {
    echo "vanilla-grub-watch: brak inotifywait (inotify-tools) — kończę" >&2
    exit 0
}
[ -d "$DIR" ] || exit 0

regen() { update-grub >/dev/null 2>&1 || true; }

# Jedno odświeżenie na starcie — wpisy aktualne po każdym rozruchu.
regen

# Reaguj na pojawianie się i znikanie katalogów migawek; drobny debounce
# scala serię zmian z jednej operacji snappera w jedno update-grub.
while inotifywait -q -r -e create -e delete -e moved_to -e moved_from "$DIR" \
        >/dev/null 2>&1; do
    sleep 3
    regen
done
