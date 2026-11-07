# vanilla-installer

Graficzny instalator **Chimera Linux** napisany w Rust (GTK4 + libadwaita),
oparty na oficjalnej procedurze z <https://chimera-linux.org/docs/installation>.

> **Autorstwo:** projekt został w całości napisany przez AI — **Claude
> (Anthropic)** — na zamówienie i według specyfikacji użytkownika.

## Co robi

* **Dwa tryby instalacji** (wybierane zaraz po uruchomieniu):
  * **z nośnika live Chimery** — używa `chimera-bootstrap -l` (z awaryjnym
    przejściem na instalację sieciową),
  * **z innego systemu** — pobiera najnowszy obraz
    `chimera-linux-<arch>-ROOTFS-<data>-bootstrap.tar.gz` z
    `repo.chimera-linux.org` (z weryfikacją SHA-256), rozpakowuje go do
    `/tmp/chimera-linux`, a następnie przez **chroot** uruchamia w nim
    `chimera-bootstrap` na docelowym dysku.
* **Proste partycjonowanie** — wybiera się tylko dysk:
  * dysk pusty → instalacja na całym dysku (GPT; na UEFI partycja ESP 1 GiB,
    na BIOS partycja „BIOS boot" 1 MiB),
  * dysk z wolnym miejscem → pytanie, czy użyć wolnego miejsca (istniejące
    partycje zostają nietknięte; istniejąca ESP jest używana ponownie),
  * brak wolnego miejsca → pytanie o przejście do **trybu zaawansowanego**
    (na razie **atrapa** — przycisk tylko informuje), dostępna jest też opcja
    wymazania całego dysku.
* **btrfs przystosowany pod snappera, identycznie jak w openSUSE**:
  * subwolumeny: `@`, `@/.snapshots`, `@/.snapshots/1/snapshot` (w nim żyje
    system; ustawiany jako *default subvolume*), `@/home`, `@/opt`, `@/root`,
    `@/srv`, `@/usr/local`, `@/var` (z `chattr +C`), `@/boot/grub`
    (odpowiednik `@/boot/grub2/*` z openSUSE — GRUB w Chimerze używa
    `/boot/grub`),
  * `/` montowany bez `subvol=` (z domyślnego subwolumenu), pozostałe
    subwolumeny jawnie w `fstab` — dokładnie jak robi to instalator openSUSE,
  * migawka nr 1 dostaje `info.xml` („first root filesystem"), więc snapper
    widzi ją od pierwszego uruchomienia,
  * `/etc/snapper/configs/root` z wartościami domyślnej instalacji openSUSE
    (`NUMBER_LIMIT="2-10"`, `NUMBER_LIMIT_IMPORTANT="4-10"`,
    `TIMELINE_CREATE="no"`, …), rejestr `SNAPPER_CONFIGS="root"`, usługa
    `snapper` (dinit) włączona,
  * GRUB z `SUSE_BTRFS_SNAPSHOT_BOOTING="true"` oraz generatorem
    `/etc/grub.d/80_snapshots`, który dodaje podmenu z rozruchem każdej
    migawki (`rootflags=subvol=@/.snapshots/N/snapshot`, tryb `ro`).
* **pkexec zamiast sudo** — GUI działa bez uprawnień; całą pracę wykonuje
  helper (`<binarka> helper`) uruchamiany przez `pkexec`, a konfiguracja
  (w tym hasła) przechodzi wyłącznie przez stdin.

## Budowanie

Wymagane: Rust (stable), `gtk4-devel`, `libadwaita-devel` (≥ 1.5).

```sh
cargo build --release
# binarka: target/release/vanilla-installer
```

Na systemach immutable (Bazzite/Silverblue) najwygodniej budować w
distroboksie z zainstalowanymi pakietami devel.

## Użycie

```sh
./vanilla-installer
```

Instalator poprosi o autoryzację polkit dopiero w momencie rozpoczęcia
instalacji. W systemie docelowym tworzony jest użytkownik w grupie `wheel`
(administracja przez `doas`, zgodnie z domyślną konfiguracją Chimery);
konto roota można zablokować, zostawiając puste hasło.

Instalowane pakiety: `base-full` (przez `chimera-bootstrap`), `linux-lts`,
`btrfs-progs`, `snapper`, `grub-x86_64-efi`+`efibootmgr` (UEFI) albo
`grub-i386-pc` (BIOS).

## Przywracanie migawek (rollback)

Układ jest w pełni zgodny ze schematem openSUSE (`snapper rollback` działa),
ale Chimera używa niełatanego GRUB-a, który w `grub.cfg` zapisuje ścieżkę
subwolumenu na sztywno. Po `snapper rollback` należy więc:

1. uruchomić ponownie komputer,
2. w menu GRUB-a wybrać *Chimera Linux — migawki btrfs* → docelową migawkę,
3. po zalogowaniu wykonać `doas update-grub`.

(openSUSE omija krok 2–3 łatkami w swoim pakiecie GRUB-a; ich przenoszenie
jest poza zakresem instalatora.)

## Ograniczenia obecnej wersji

* Tryb zaawansowanego partycjonowania to atrapa (zgodnie z założeniem).
* Architektura docelowa: ta sama, na której działa instalator (testowane
  założenia: x86_64; tryb BIOS/UEFI wykrywany z hosta).
* Brak tworzenia partycji wymiany (swap) i wyboru środowiska graficznego —
  po instalacji system to `base-full` (konsola); desktop doinstalowuje się
  przez `apk`.
* Strefa czasowa i układ klawiatury nie mają osobnych stron — strefa jest
  polem tekstowym (domyślnie wykrywana z hosta).

## Licencja i pochodzenie konfiguracji

Kod: licencja MIT (patrz `LICENSE`).

Konfiguracja snappera/GRUB-a **odwzorowuje wartości i układ** znane z
openSUSE (github.com/openSUSE); są to dane konfiguracyjne, nie kod. Skrypt
`80_snapshots` został napisany od zera na potrzeby tego projektu i **nie
zawiera kodu** z `grub2-snapper-plugin` ani innych źródeł GPL — odtwarza
jedynie zachowanie. Narzędzia Chimery (`chimera-bootstrap`,
`chimera-chroot`, `apk`, `update-grub`) są wyłącznie **wywoływane**.
