#!/bin/sh
# chimera-grub-watch — watches the snapshot directory and automatically
# refreshes the GRUB menu (update-grub) after every snapshot creation or
# deletion, so the "btrfs snapshots" submenu is always current. The
# grub-btrfs.path equivalent from CachyOS, adapted to dinit. Written from
# scratch for chimera-installer.
set -u

DIR=/.snapshots

command -v inotifywait >/dev/null 2>&1 || {
    echo "chimera-grub-watch: inotifywait (inotify-tools) missing — exiting" >&2
    exit 0
}
[ -d "$DIR" ] || exit 0

regen() { update-grub >/dev/null 2>&1 || true; }

# One refresh at startup — entries current after every boot.
regen

# React to snapshot directories appearing and disappearing; a small debounce
# merges a burst of changes from one snapper operation into a single
# update-grub.
while inotifywait -q -r -e create -e delete -e moved_to -e moved_from "$DIR" \
        >/dev/null 2>&1; do
    sleep 3
    regen
done
