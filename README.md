# chimera-installer

Graficzny instalator **Chimera Linux** napisany w Rust (GTK4 + libadwaita),
oparty na oficjalnej procedurze z <https://chimera-linux.org/docs/installation>.

> **Autorstwo:** projekt został w całości napisany przez AI — **Claude
> (Anthropic)** — na zamówienie i według specyfikacji użytkownika.

## Co robi

* **Dwa tryby instalacji** (wybierane zaraz po uruchomieniu):
  * **z nośnika live Chimery** — z wyborem źródła pakietów:
    * *lokalnie* (`chimera-bootstrap -f -l`) — wierna kopia systemu live
      (razem z pulpitem nośnika), jądro dziedziczone z nośnika; usługi
      włączone na live (`/etc/dinit.d/boot.d`) są odwzorowywane w systemie
      docelowym; snapper i mikrokod CPU są dobierane z sieci,
    * *z sieci* (`chimera-bootstrap -f`) — najnowsze pakiety z repozytorium
      oraz wybór systemu: **system bazowy / GNOME / KDE Plasma**, a dla
      pulpitów dodatkowo wariant **base** (bez aplikacji, z samym terminalem:
      `apk add gnome '!gnome-apps'` + `gnome-console`, albo `plasma-desktop`
      bez `*-apps-meta`/`*-multimedia-meta` + `konsole`) albo **full**
      (pełny metapakiet: `apk add gnome` / `apk add plasma-desktop`),
  * **z innego systemu** — pobiera najnowszy obraz
    `chimera-linux-<arch>-ROOTFS-<data>-bootstrap.tar.gz` z
    `repo.chimera-linux.org` (z weryfikacją SHA-256), rozpakowuje go do
    `/tmp/chimera-linux`, a następnie przez **chroot** uruchamia w nim
    `chimera-bootstrap` na docelowym dysku; działa zawsze jak instalacja
    sieciowa (z wyborem base / GNOME / KDE).
* **System gotowy po pierwszym rozruchu** — usługi dinit włączane są
  w chroocie dowiązaniami w `/etc/dinit.d/boot.d/` (dokładnie tak, jak robi
  to `dinitctl enable`, który w chroocie nie działa — wymaga żywego demona):
  `syslog-ng`, demon sieci (`dhcpcd` dla base, `networkmanager` dla
  pulpitów), menedżer logowania (`gdm`/`sddm`) i `snapper`. Usług włączanych
  domyślnie przez pakiety `*-dinit-links` (dbus, elogind, chrony,
  turnstiled, udevd, pipewire/wireplumber per-user) instalator nie dubluje.
  Do instalacji sieciowej dokładany jest jawnie `dinit-chimera-udev`
  (base-full zostawia atrapę `device-none` bez udevd) oraz mikrokod CPU
  (`ucode-amd`/`ucode-intel` wg procesora).
* **Proste partycjonowanie** — wybiera się tylko dysk:
  * dysk pusty → instalacja na całym dysku (GPT; na UEFI partycja ESP 1 GiB,
    na BIOS partycja „BIOS boot" 1 MiB),
  * dysk z wolnym miejscem → pytanie, czy użyć wolnego miejsca (istniejące
    partycje zostają nietknięte; istniejąca ESP jest używana ponownie),
  * brak wolnego miejsca → pytanie o przejście do **trybu zaawansowanego**
    (na razie **atrapa** — przycisk tylko informuje), dostępna jest też opcja
    wymazania całego dysku.
* **btrfs przystosowany pod snappera w układzie CachyOS** (świadomie zamiast
  openSUSE — rollback działa wtedy na **waniliowym GRUB**, bez łatania
  bootloadera i z samej konsoli):
  * subwolumeny płaskie, top-level: `@` (root), `@home`, `@root`, `@srv`,
    `@cache` (/var/cache), `@tmp` (/var/tmp), `@log` (/var/log, z `chattr +C`
    jak `@cache`), oraz `@snapshots` na `/.snapshots`,
  * `/` montowany jako **stały `subvol=@`** — to on sprawia, że rollback
    (podmiana zawartości `@`) jest widoczny dla niełatanego GRUB-a,
  * `/boot` zostaje **wewnątrz `@`**, więc rollback obejmuje też kernel/initrd
    (lepiej niż CachyOS z Limine, gdzie kernel jest na ESP poza migawkami),
  * `@snapshots` jest **osobnym** subwolumenem (poza `@`), żeby migawki
    przeżyły podmianę `@` bez żadnej specjalnej obsługi,
  * `/etc/snapper/configs/root` (`NUMBER_LIMIT="2-10"`, `TIMELINE_CREATE="no"`,
    …), rejestr `SNAPPER_CONFIGS="root"`, usługa `snapper` (dinit) włączona;
    tworzona jest migawka wyjściowa „instalacja systemu",
  * generator `/etc/grub.d/80_snapshots` dodaje podmenu z rozruchem migawek
    **read-only** (ścieżki od top-level `/@snapshots/N/snapshot/…`, działa na
    waniliowym GRUB) — ścieżka ratunkowa, gdy system nie wstaje,
  * usługa dinit `chimera-grub-watch` (przez `inotify-tools`) pilnuje
    `/.snapshots` i **automatycznie wykonuje `update-grub` po każdym
    utworzeniu lub usunięciu migawki** (odpowiednik `grub-btrfs.path`
    z CachyOS), więc podmenu migawek jest zawsze aktualne,
  * po zakończonej instalacji tworzona jest **chroniona migawka „FABRIC"**
    (nr 1) — stan systemu jak po pierwszym uruchomieniu; robiona wprost
    narzędziami btrfs (read-only), z pustym `cleanup`, więc automatyczne
    czyszczenie snappera jej nie usuwa (`doas chimera-rollback 1` wraca do
    stanu fabrycznego). Dodatkowo katalog migawki dostaje `chattr +i`, co
    blokuje `snapper delete`/`btrfs subvolume delete`, dopóki ktoś świadomie
    nie zdejmie immutable (`doas chattr -i /.snapshots/1`) — to utrudnienie,
    nie absolutna blokada (root zawsze może je cofnąć). W **głównym menu** GRUB
    pojawia się wpis **„Restore factory settings"** (generator
    `/etc/grub.d/12_fabric`), który uruchamia migawkę FABRIC w trybie read-only
    (podgląd stanu fabrycznego; trwałe przywrócenie z niej przez
    `doas chimera-rollback 1`). Generator leży w `@`, więc jest w każdej
    migawce — po dowolnym rollbacku, który przywraca `grub.cfg` razem z `@`,
    wpis nadal powstaje; zawsze aktualny, bo migawka nr 1 jest niezmienna.
* **Rollback z wiersza poleceń** — narzędzie `chimera-rollback` (instalowane
  do `/usr/bin`) przywraca migawkę metodą podmiany `@` (jak CachyOS/Timeshift),
  **bez GUI i bez sterownika graficznego**: `doas chimera-rollback list`,
  `doas chimera-rollback <N>`, `reboot`. Działa z konsoli `tty`, przez SSH,
  z bootniętej migawki read-only, a nawet z live USB. `snapper rollback`
  (openSUSE, oparty na `set-default`) świadomie NIE jest używany — waniliowy
  GRUB by go zignorował.
* **Strona „System" i „Użytkownik"** — nazwa komputera, strefa czasowa
  (dialog kontynent → strefa), język (`LANG` w `/etc/locale.conf`, działa też
  w tty), układ klawiatury (dialog układ + wariant; zapis do
  `/etc/default/keyboard` dla konsoli i `/etc/X11/xorg.conf.d/00-keyboard.conf`
  dla sesji graficznej) oraz przełącznik **serwera SSH** (sshd — domyślnie
  wyłączony, także gdy nośnik live miał go włączonego); konto użytkownika
  i roota na osobnej stronie.
* **pkexec zamiast sudo** — GUI działa bez uprawnień; całą pracę wykonuje
  helper (`<binarka> helper`) uruchamiany przez `pkexec`, a konfiguracja
  (w tym hasła) przechodzi wyłącznie przez stdin.

## Budowanie

Wymagane: Rust (stable), `gtk4-devel`, `libadwaita-devel` (≥ 1.5).

```sh
cargo build --release
# binarka: target/release/chimera-installer
```

Na systemach immutable (Bazzite/Silverblue) najwygodniej budować w
distroboksie z zainstalowanymi pakietami devel.

## Użycie

```sh
./chimera-installer
```

Instalator poprosi o autoryzację polkit dopiero w momencie rozpoczęcia
instalacji. W systemie docelowym tworzony jest użytkownik w grupie `wheel`
(administracja przez `doas`, zgodnie z domyślną konfiguracją Chimery);
konto roota można zablokować, zostawiając puste hasło. Dla wygody tworzone
jest dowiązanie `/usr/bin/sudo` → `doas`, więc polecenia `sudo …` też działają.

Instalowane pakiety: `base-full` (przez `chimera-bootstrap`), `linux-stable`
(poza trybem lokalnym — tam jądro pochodzi z kopii live), `btrfs-progs`,
`snapper` (repozytorium `user` dodawane automatycznie), `openssl`,
`dinit-chimera-udev`, mikrokod CPU, `grub-x86_64-efi`+`efibootmgr` (UEFI)
albo `grub-i386-pc` (BIOS); dla pulpitów dodatkowo `gnome`+`gdm` lub
`plasma-desktop`+`sddm` oraz `networkmanager`.

## Eksperymentalne: zastrzeżony sterownik NVIDIA

Dla kart NVIDIA, **wyłącznie w trybie live**, na ekranie karty graficznej
dostępne są dodatkowo zastrzeżone sterowniki (gałąź **620** — Turing i nowsze,
albo **580** — Maxwell/Pascal). Ponieważ pakiety nie są jeszcze w repozytorium
Chimery, instalator **buduje je na żywym nośniku** z otwartego PR-a cports
(#6531) narzędziem `cbuild`:

1. doinstalowuje zależnoości budowania (`base-cbuild-host`, `ruff`),
2. klonuje cports do `/tmp/musl-nvidia` (aktualny tip gałęzi PR-a; PR bywa
   force-pushowany, więc sztywny commit szybko przestaje być osiągalny —
   użyty SHA trafia do dziennika),
3. buduje pakiety jako zwykły użytkownik (cbuild odmawia pracy jako root),
4. instaluje wynikowe `.apk` do systemu docelowego (`apk --root
   --allow-untrusted`), razem z `linux-stable-devel` i `*-ckms`; moduł jądra
   buduje trigger ckms, a `musl` jest podmieniany na `musl-nvidia` w tej samej
   transakcji; blacklistę nouveau i firmware GSP dostarczają same pakiety.

Opcja jest **eksperymentalna**: PR jest oznaczony jako roboczy, build pobiera
~1,5–2 GB i zajmuje kilka GB miejsca (uwaga na `tmpfs` w `/tmp` na nośniku
live), a z powodu znanego błędu apk moduł jądra może zbudować się dopiero przy
pierwszym uruchomieniu. Przed wyborem instalator wyświetla ostrzeżenie.

## Przywracanie migawek (rollback)

Rollback działa **w całości z wiersza poleceń**, bez GUI i bez sterownika
graficznego, na niełatanym GRUB-ie — bo `/` to stały `subvol=@`, a przywrócenie
podmienia *zawartość* `@` (model CachyOS/Timeshift):

1. z działającego systemu albo z konsoli `tty` (gdy padł tylko pulpit),
   przez SSH, z bootniętej migawki read-only lub z live USB:
   ```sh
   doas chimera-rollback list      # numery dostępnych migawek
   doas chimera-rollback 7         # przywróć migawkę nr 7
   reboot
   ```
2. po restarcie GRUB montuje stały `subvol=@`, w którym jest już przywrócona
   zawartość — **żadnego `update-grub` ani wyboru w menu**.

Poprzedni stan zostaje zachowany jako `@_rollback_backup_<data>` (można go
później usunąć). `snapper rollback` z openSUSE (oparty na `set-default`)
celowo NIE jest używany, bo waniliowy GRUB by go zignorował.

Ręcznie z live USB (gdy nie wstaje nawet konsola) — to samo, co robi
`chimera-rollback`:
```sh
doas mount -o subvolid=5 /dev/<partycja-btrfs> /mnt
doas mv /mnt/@ /mnt/@_backup
doas btrfs subvolume snapshot /mnt/@snapshots/<N>/snapshot /mnt/@
doas umount /mnt && reboot
```

## Ograniczenia obecnej wersji

* Tryb zaawansowanego partycjonowania to atrapa (zgodnie z założeniem).
* Architektura docelowa: ta sama, na której działa instalator (testowane
  założenia: x86_64; tryb BIOS/UEFI wykrywany z hosta).
* Brak tworzenia partycji wymiany (swap).
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
