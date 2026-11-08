#!/bin/sh
# Osier installer: curl -fsSL https://raw.githubusercontent.com/ReyArlena/osier/main/install.sh | sh
#
# Installs the latest release binary to ~/.local/bin (or $OSIER_PREFIX/bin), with the desktop
# entry and icons under ~/.local/share. Falls back to building that same release from source
# with cargo when no prebuilt binary matches this machine (or the download fails), and to main
# only while no release exists. Set OSIER_VERSION=v1.2.3 to pin a release, OSIER_VERSION=main
# for the latest code, and OSIER_FROM_SOURCE=1 to force a source build.
set -eu

REPO="ReyArlena/osier"
REPO_URL="https://github.com/$REPO.git"
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

# Newest stable release tag (vX.Y.Z, no pre-releases), read from the repository itself so it
# works when the GitHub API is rate limited. Prints nothing when there is no release yet.
latest_tag() {
  have git || return 0
  GIT_TERMINAL_PROMPT=0 git ls-remote --tags --refs "$REPO_URL" 2>/dev/null \
    | sed -n 's|.*refs/tags/v\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)$|\1|p' \
    | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1 | sed 's/^/v/'
}

install_release() {
  [ -n "$target" ] || return 1
  [ "${OSIER_VERSION:-}" = "main" ] && return 1
  version="$(resolve_version)" || return 1
  plain="${version#v}"
  name="osier-${plain}-${target}"
  base="https://github.com/$REPO/releases/download/$version"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  say "Downloading $name.tar.gz"
  download "$base/$name.tar.gz" "$work/$name.tar.gz" || return 1
  download "$base/$name.tar.gz.sha256" "$work/$name.tar.gz.sha256" || return 1
  (cd "$work" && sha256sum -c "$name.tar.gz.sha256" >/dev/null) || die "checksum mismatch for $name.tar.gz"
  tar -C "$work" -xzf "$work/$name.tar.gz"
  install_files "$work/$name"
  say "Installed osier $plain to $BIN_DIR/osier"
}

install_from_source() {
  have cargo || die "no prebuilt binary for this machine and cargo is not installed (https://rustup.rs)"
  have git || die "git is required to build from source"
  say "Building from source (this takes a few minutes)"
  if have pacman; then
    say "Build dependencies on Arch: pkgconf libxkbcommon libxkbcommon-x11 wayland vulkan-headers fontconfig openssl"
  elif have apt-get; then
    say "Build dependencies on Debian/Ubuntu: pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb-xkb-dev libwayland-dev libvulkan-dev libfontconfig1-dev libssl-dev"
  elif have dnf; then
    say "Build dependencies on Fedora: pkgconf-pkg-config libxkbcommon-devel libxkbcommon-x11-devel wayland-devel vulkan-loader-devel fontconfig-devel openssl-devel"
  fi
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  ref="${OSIER_VERSION:-$(latest_release_tag)}"
  [ -n "$ref" ] || ref="$(latest_tag)"
  if [ -z "$ref" ]; then
    ref="main"
    say "No release yet: building the latest code from main"
  elif [ "$ref" != "main" ]; then
    say "Building release $ref"
  fi
  git clone --quiet --depth 1 --branch "$ref" "$REPO_URL" "$work/src"
  cargo install --locked --path "$work/src/crates/osier" --root "$PREFIX"
  install_desktop "$work/src/packaging"
  say "Installed osier to $BIN_DIR/osier"
}

check_runtime_deps
if [ "${OSIER_FROM_SOURCE:-0}" = "1" ] || ! install_release; then
  install_from_source
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "note: $BIN_DIR is not on your PATH" ;;
esac
