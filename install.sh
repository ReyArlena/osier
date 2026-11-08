#!/bin/sh
# Osier installer: curl -fsSL https://raw.githubusercontent.com/ReyArlena/osier/main/install.sh | sh
#
# Installs the latest release binary to ~/.local/bin (or $OSIER_PREFIX/bin), with the desktop
# entry and icons under ~/.local/share. Prebuilt binaries only: x86_64 and ARM64 Linux with
# glibc 2.35 or newer. Anything else stops with how to build from source instead. Set
# OSIER_VERSION=v1.2.3 to install a specific release.
set -eu

REPO="ReyArlena/osier"
# The release binaries are built on Ubuntu 22.04 (see .github/workflows/release.yml).
MIN_GLIBC="2.35"
PREFIX="${OSIER_PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_ID="com.reyarlena.osier"

say() { printf '%s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
  *) target="" ;;
esac
[ "$(uname -s)" = "Linux" ] || die "Osier runs on Linux only"

# No prebuilt binary fits this machine: say so, and how to get Osier anyway.
no_build() {
  say "error: $1"
  say ""
  say "There is no prebuilt Osier for this system. You can build it from source:"
  say "  https://github.com/$REPO#install"
  say "or ask for a build for your system:"
  say "  https://github.com/$REPO/issues/new"
  exit 1
}

# The binaries link glibc; musl systems and older glibc cannot run them.
check_platform() {
  [ -n "$target" ] || no_build "unsupported CPU architecture: $arch"
  libc="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"   # "glibc 2.39"
  if [ -z "$libc" ]; then
    if ldd --version 2>&1 | grep -qi musl; then
      no_build "this system uses musl libc; the prebuilt binaries need glibc $MIN_GLIBC or newer"
    fi
    say "warning: could not tell the C library version; the binary needs glibc $MIN_GLIBC or newer"
    return 0
  fi
  have_v="${libc##* }"
  have_major="${have_v%%.*}"; have_minor="${have_v#*.}"; have_minor="${have_minor%%.*}"
  need_major="${MIN_GLIBC%%.*}"; need_minor="${MIN_GLIBC#*.}"
  if [ "$have_major" -lt "$need_major" ] ||
     { [ "$have_major" -eq "$need_major" ] && [ "$have_minor" -lt "$need_minor" ]; }; then
    no_build "this system has glibc $have_v; the prebuilt binaries need glibc $MIN_GLIBC or newer"
  fi
}

check_runtime_deps() {
  # Shared libraries loaded at runtime. Missing ones only produce a warning: the user may
  # have them under a name ldconfig does not list.
  missing=""
  for lib in libxkbcommon.so.0 libwayland-client.so.0 libvulkan.so.1 libfontconfig.so.1; do
    if ! ldconfig -p 2>/dev/null | grep -q "$lib"; then
      missing="$missing $lib"
    fi
  done
  if [ -n "$missing" ]; then
    say "warning: runtime libraries not found:$missing"
    if have pacman; then
      say "  pacman -S libxkbcommon wayland vulkan-icd-loader fontconfig"
    elif have apt-get; then
      say "  apt install libxkbcommon0 libwayland-client0 libvulkan1 libfontconfig1"
    elif have dnf; then
      say "  dnf install libxkbcommon wayland vulkan-loader fontconfig"
    fi
  fi
  have git || say "warning: git is not on PATH; Osier needs git 2.30 or newer"
}

download() {
  url="$1"; out="$2"
  if have curl; then curl -fsSL "$url" -o "$out"
  elif have wget; then wget -qO "$out" "$url"
  else die "need curl or wget"
  fi
}

# Newest published release (not a draft or pre-release): github.com/<repo>/releases/latest
# redirects to .../releases/tag/vX.Y.Z. A plain web redirect, so the API rate limit does not
# apply. Prints nothing when there is no release yet or GitHub is unreachable.
latest_release_tag() {
  url="https://github.com/$REPO/releases/latest"
  if have curl; then
    curl -fsSLI -o /dev/null -w '%{url_effective}' "$url" 2>/dev/null
  elif have wget; then
    wget -q -S --spider --max-redirect=0 "$url" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p'
  fi | sed -n 's|.*/releases/tag/\(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)[[:space:]]*$|\1|p' | head -n 1
}

resolve_version() {
  if [ -n "${OSIER_VERSION:-}" ]; then
    printf '%s' "$OSIER_VERSION"
    return
  fi
  v="$(latest_release_tag)"
  [ -n "$v" ] || return 1
  printf '%s' "$v"
}

install_files() {
  src="$1"
  mkdir -p "$BIN_DIR"
  install -m 755 "$src/osier" "$BIN_DIR/osier"
  install_desktop "$src"
}

# Desktop entry, AppStream metadata and icons. Exec points at the installed binary, so the
# launcher works even when the desktop session's PATH lacks $BIN_DIR.
install_desktop() {
  src="$1"
  if [ -f "$src/$APP_ID.desktop" ]; then
    mkdir -p "$DATA_DIR/applications" "$DATA_DIR/metainfo"
    # The main entry, plus a hidden one matching the askpass window's app id (for its icon).
    for id in "$APP_ID" "$APP_ID.askpass"; do
      [ -f "$src/$id.desktop" ] || continue
      sed -e "s|^Exec=osier |Exec=$BIN_DIR/osier |" -e "s|^Exec=osier\$|Exec=$BIN_DIR/osier|" \
        "$src/$id.desktop" > "$DATA_DIR/applications/$id.desktop"
      chmod 644 "$DATA_DIR/applications/$id.desktop"
    done
    [ -f "$src/$APP_ID.metainfo.xml" ] && install -m 644 "$src/$APP_ID.metainfo.xml" "$DATA_DIR/metainfo/$APP_ID.metainfo.xml"
    for dir in "$src"/icons/hicolor/*; do
      size="$(basename "$dir")"
      mkdir -p "$DATA_DIR/icons/hicolor/$size/apps"
      cp "$dir"/apps/* "$DATA_DIR/icons/hicolor/$size/apps/" 2>/dev/null || true
    done
    have update-desktop-database && update-desktop-database "$DATA_DIR/applications" 2>/dev/null || true
    have gtk-update-icon-cache && gtk-update-icon-cache -q "$DATA_DIR/icons/hicolor" 2>/dev/null || true
  fi
}

install_release() {
  version="$(resolve_version)" ||
    die "no Osier release found (none published yet, or GitHub is unreachable)"
  plain="${version#v}"
  name="osier-${plain}-${target}"
  base="https://github.com/$REPO/releases/download/$version"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  say "Downloading $name.tar.gz"
  download "$base/$name.tar.gz" "$work/$name.tar.gz" ||
    die "could not download $name.tar.gz (is $version a published release?)"
  download "$base/$name.tar.gz.sha256" "$work/$name.tar.gz.sha256" ||
    die "could not download the checksum for $name.tar.gz"
  (cd "$work" && sha256sum -c "$name.tar.gz.sha256" >/dev/null) || die "checksum mismatch for $name.tar.gz"
  tar -C "$work" -xzf "$work/$name.tar.gz"
  install_files "$work/$name"
  say "Installed osier $plain to $BIN_DIR/osier"
}

check_platform
check_runtime_deps
install_release

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "note: $BIN_DIR is not on your PATH" ;;
esac
