#!/bin/sh
# Osier installer: curl -fsSL https://raw.githubusercontent.com/ReyArlena/osier/main/install.sh | sh
#
# Installs the latest release binary to ~/.local/bin (or $OSIER_PREFIX/bin), with the desktop
# entry and icons under ~/.local/share. Prebuilt binaries only: x86_64 and ARM64 Linux with
# glibc 2.35 or newer. Anything else stops with how to build from source instead. Set
# OSIER_VERSION=v1.2.3 to install a specific release.
#
# `osier --update` and Settings > About > Updates run this script too. In a terminal it shows
# each step with a tick, then the commands to start with. Anywhere else (the app captures the
# output, a log file) it prints plain lines only.
set -eu

REPO="ReyArlena/osier"
# The release binaries are built on Ubuntu 22.04 (see .github/workflows/release.yml).
MIN_GLIBC="2.35"
PREFIX="${OSIER_PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_ID="com.reyarlena.osier"

say() { printf '%s\n' "$*" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }

# An Osier already in place means this run replaces it.
updating=0
if [ -x "$BIN_DIR/osier" ]; then updating=1; fi

# Steps, colour and the closing notes are for a person at a terminal.
pretty=0
if [ -t 2 ]; then pretty=1; fi
BOLD="" DIM="" RED="" GREEN="" YELLOW="" RESET="" PAD=""
ARROW="->" TICK="ok" CROSS="failed" MARK=">" DOTS="..."
if [ "$pretty" = 1 ]; then
  PAD=" "
  case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
    *[Uu][Tt][Ff]-8*|*[Uu][Tt][Ff]8*) ARROW="→" TICK="✓" CROSS="✗" MARK="▸" DOTS="…" ;;
  esac
  if [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != "dumb" ]; then
    esc="$(printf '\033')"
    BOLD="${esc}[1m" DIM="${esc}[2m" RED="${esc}[31m" GREEN="${esc}[32m"
    YELLOW="${esc}[33m" RESET="${esc}[0m"
  fi
fi

in_step=0     # a step line is waiting for its tick
tool_err=""   # what curl, wget or tar said when they failed
work=""       # the download folder, removed on exit

# Finish the waiting step line with a mark: end_step <colour> <mark>
end_step() {
  if [ "$in_step" = 1 ]; then
    printf '%s%s%s\n' "$1" "$2" "$RESET" >&2
    in_step=0
  fi
}
heading() {
  if [ "$pretty" = 1 ]; then printf '\n %s%s%s %s%s\n\n' "$BOLD" "$GREEN" "$MARK" "$1" "$RESET" >&2; fi
}
# step "Doing something" ... ok ["detail"]. A warning or an error in between ends the line.
step() {
  if [ "$pretty" = 1 ]; then
    printf ' %s%s %s%s%s ' "$DIM" "$ARROW" "$1" "$DOTS" "$RESET" >&2
    in_step=1
  fi
}
ok() {
  if [ "$in_step" = 1 ]; then
    printf '%s%s%s' "$GREEN" "$TICK" "$RESET" >&2
    if [ -n "${1:-}" ]; then printf '  %s%s%s' "$DIM" "$1" "$RESET" >&2; fi
    printf '\n' >&2
    in_step=0
  fi
}
# A line for the plain output only.
note() {
  if [ "$pretty" = 0 ]; then say "$*"; fi
}
warn() {
  if [ "$pretty" = 1 ]; then
    end_step "$YELLOW" "!"
    say "   ${YELLOW}$*${RESET}"
  else
    say "warning: $*"
  fi
}
die() {
  end_step "$RED" "$CROSS"
  say "$PAD${RED}error:${RESET} $*"
  if [ -n "$tool_err" ]; then say "$PAD$tool_err"; fi
  exit 1
}
# The home folder as ~, for display.
tilde() {
  case "$1" in
    "$HOME"/*) printf '%s/%s' '~' "${1#"$HOME"/}" ;;
    *) printf '%s' "$1" ;;
  esac
}

# Whatever stops the script, close the open step line and remove the download folder.
cleanup() {
  end_step "$RED" "$CROSS"
  if [ -n "$work" ]; then rm -rf "$work"; fi
}
trap cleanup EXIT

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
  *) target="" ;;
esac

# No prebuilt binary fits this machine: say so, and how to get Osier anyway.
no_build() {
  end_step "$RED" "$CROSS"
  say "$PAD${RED}error:${RESET} $1"
  say ""
  say "${PAD}There is no prebuilt Osier for this system. You can build it from source:"
  say "$PAD  https://github.com/$REPO#install"
  say "${PAD}or ask for a build for your system:"
  say "$PAD  https://github.com/$REPO/issues/new"
  exit 1
}

# The binaries link glibc; musl systems and older glibc cannot run them.
check_platform() {
  step "Checking system"
  [ "$(uname -s)" = "Linux" ] || die "Osier runs on Linux only"
  [ -n "$target" ] || no_build "unsupported CPU architecture: $arch"
  libc="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"   # "glibc 2.39"
  if [ -z "$libc" ]; then
    if ldd --version 2>&1 | grep -qi musl; then
      no_build "this system uses musl libc; the prebuilt binaries need glibc $MIN_GLIBC or newer"
    fi
    warn "could not tell the C library version; the binary needs glibc $MIN_GLIBC or newer"
    return 0
  fi
  have_v="${libc##* }"
  have_major="${have_v%%.*}"; have_minor="${have_v#*.}"; have_minor="${have_minor%%.*}"
  need_major="${MIN_GLIBC%%.*}"; need_minor="${MIN_GLIBC#*.}"
  if [ "$have_major" -lt "$need_major" ] ||
     { [ "$have_major" -eq "$need_major" ] && [ "$have_minor" -lt "$need_minor" ]; }; then
    no_build "this system has glibc $have_v; the prebuilt binaries need glibc $MIN_GLIBC or newer"
  fi
  ok "$arch, glibc $have_v"
}

check_runtime_deps() {
  # Shared libraries loaded at runtime. Missing ones only produce a warning: the user may
  # have them under a name ldconfig does not list.
  step "Checking runtime libraries and git"
  missing=""; pacman_pkgs=""; apt_pkgs=""; dnf_pkgs=""
  for lib in libxkbcommon.so.0 libwayland-client.so.0 libvulkan.so.1 libfontconfig.so.1; do
    if ldconfig -p 2>/dev/null | grep -q "$lib"; then continue; fi
    missing="$missing $lib"
    case "$lib" in
      libxkbcommon*) pacman_pkg=libxkbcommon apt_pkg=libxkbcommon0 dnf_pkg=libxkbcommon ;;
      libwayland*) pacman_pkg=wayland apt_pkg=libwayland-client0 dnf_pkg=wayland ;;
      libvulkan*) pacman_pkg=vulkan-icd-loader apt_pkg=libvulkan1 dnf_pkg=vulkan-loader ;;
      *) pacman_pkg=fontconfig apt_pkg=libfontconfig1 dnf_pkg=fontconfig ;;
    esac
    pacman_pkgs="$pacman_pkgs $pacman_pkg"; apt_pkgs="$apt_pkgs $apt_pkg"; dnf_pkgs="$dnf_pkgs $dnf_pkg"
  done
  if [ -n "$missing" ]; then
    warn "runtime libraries not found:$missing"
    if have pacman; then
      say "$PAD  pacman -S$pacman_pkgs"
    elif have apt-get; then
      say "$PAD  apt install$apt_pkgs"
    elif have dnf; then
      say "$PAD  dnf install$dnf_pkgs"
    fi
  fi
  have git || warn "git is not on PATH; Osier needs git 2.30 or newer"
  ok
}

download() {
  url="$1"; out="$2"
  # Their own error text is kept for die, so it does not land on the step line.
  if have curl; then tool_err="$(curl -fsSL "$url" -o "$out" 2>&1)"
  elif have wget; then tool_err="$(wget -qO "$out" "$url" 2>&1)"
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

install_files() {
  src="$1"
  step "Installing to $(tilde "$BIN_DIR")"
  mkdir -p "$BIN_DIR"
  install -m 755 "$src/osier" "$BIN_DIR/osier"
  ok
  install_desktop "$src"
}

# Desktop entry, AppStream metadata and icons. Exec points at the installed binary, so the
# launcher works even when the desktop session's PATH lacks $BIN_DIR.
install_desktop() {
  src="$1"
  if [ -f "$src/$APP_ID.desktop" ]; then
    step "Adding desktop entry and icons"
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
    ok
  fi
}

install_release() {
  if [ -n "${OSIER_VERSION:-}" ]; then
    version="$OSIER_VERSION"
    step "Using release $version"
    ok
  else
    step "Finding latest release"
    version="$(latest_release_tag)"
    [ -n "$version" ] || die "no Osier release found (none published yet, or GitHub is unreachable)"
    ok "$version"
  fi
  plain="${version#v}"
  name="osier-${plain}-${target}"
  base="https://github.com/$REPO/releases/download/$version"
  have sha256sum || die "need sha256sum to check the download"
  work="$(mktemp -d)"
  step "Downloading $name.tar.gz"
  note "Downloading $name.tar.gz"
  download "$base/$name.tar.gz" "$work/$name.tar.gz" ||
    die "could not download $name.tar.gz (is $version a published release?)"
  download "$base/$name.tar.gz.sha256" "$work/$name.tar.gz.sha256" ||
    die "could not download the checksum for $name.tar.gz"
  ok
  step "Verifying checksum"
  (cd "$work" && sha256sum -c "$name.tar.gz.sha256" >/dev/null 2>&1) ||
    die "checksum mismatch for $name.tar.gz"
  ok
  tool_err="$(tar -C "$work" -xzf "$work/$name.tar.gz" 2>&1)" || die "could not unpack $name.tar.gz"
  install_files "$work/$name"
  note "Installed osier $plain to $BIN_DIR/osier"
}

# One line of the command list: usage "command" "what it does"
usage() {
  printf '   %-28s%s%s%s\n' "$1" "$DIM" "$2" "$RESET" >&2
}

# What to do next, for a person at a terminal.
closing_notes() {
  say ""
  say " ${BOLD}${GREEN}$TICK Osier $plain installed${RESET}"
  if [ "$updating" = 1 ]; then
    say ""
    say "   $RESTART"
  fi
  if [ "$on_path" = 0 ]; then
    say ""
    say " ${YELLOW}! $(tilde "$BIN_DIR") is not on your PATH${RESET}"
  fi

  heading "Getting started"
  usage "osier" "open Osier with your last session"
  usage "osier <path>" "open a repository"
  usage "osier -n <path>" "open it in a separate window"
  usage "osier diff <left> <right>" "compare two files"
  usage "osier merge <left> [<base>] <right> <output>" ""
  usage "" "merge two versions of a file"
  usage "osier --update" "install the latest release"
  usage "osier --help" "all options"
  say ""
  say "   Osier is in your app launcher too. To use it as git's mergetool and difftool:"
  say "   https://github.com/$REPO#git-mergetool-and-difftool"

  heading "Thank you"
  say "   If you enjoy using Osier, please show your support by adding a Star on"
  say "   GitHub, or opening an Issue if you have feedback. Thank you!"
  say ""
  say "   https://github.com/$REPO"
  say ""
}

RESTART="If you have an instance of Osier running, we recommend you restart it."

if [ "$updating" = 1 ]; then heading "Updating Osier"; else heading "Installing Osier"; fi
check_platform
check_runtime_deps
install_release

on_path=0
case ":$PATH:" in
  *":$BIN_DIR:"*) on_path=1 ;;
esac

if [ "$pretty" = 1 ]; then
  closing_notes
else
  if [ "$updating" = 1 ]; then say "$RESTART"; fi
  if [ "$on_path" = 0 ]; then say "note: $BIN_DIR is not on your PATH"; fi
fi
