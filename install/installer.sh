#!/bin/sh
# Installs femp with everything it needs: the femp binary, which has the Fe
# compiler built in, the Z3 solver `femp prove` uses, Node.js (for
# WebAssembly) when no Node.js 20 or newer is found, and a C compiler (for
# native executables) when none is found.
#
#   curl -fsSL https://github.com/emeieiron/femp/releases/latest/download/installer.sh | sh
#
# Options, passed as `sh -s -- <options>` when piping:
#   --yes             install a missing C compiler without asking
#   --no-modify-path  do not add femp to PATH in shell startup files
#   --skip-deps       install femp and Z3 only
#
# Environment:
#   FEMP_HOME          where to install (default: ~/.femp)
#   FEMP_VERSION       release tag to install, such as v0.2.0 (default: the latest)
#   FEMP_DOWNLOAD_URL  base URL of the release files, for mirrors and testing
#   FEMP_NODE_MIRROR   base URL of Node.js releases (default: https://nodejs.org/dist)
#
# Node.js is installed under FEMP_HOME for femp alone and does not change
# which `node` other programs see. Installing a C compiler uses the system
# package manager and may ask for your password.
set -eu

REPOSITORY="emeieiron/femp"
NODE_VERSION="24.16.0"
NODE_MIN_MAJOR=20

yes=false
modify_path=true
deps=true

say() {
    printf '%s\n' "$*"
}

warn() {
    printf 'warning: %s\n' "$*" >&2
}

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

has() {
    command -v "$1" >/dev/null 2>&1
}

# Questions go to the terminal: when piped into sh, standard input is the
# script itself.
interactive() {
    (exec </dev/tty) 2>/dev/null
}

confirm() {
    if [ "$yes" = true ]; then
        return 0
    fi
    if ! interactive; then
        return 1
    fi
    printf '%s [Y/n] ' "$1" >/dev/tty
    read -r answer </dev/tty || return 1
    case "$answer" in
        "" | y | Y | yes | Yes) return 0 ;;
        *) return 1 ;;
    esac
}

download() {
    if has curl; then
        curl --proto '=https,file' --tlsv1.2 -fsSL "$1" -o "$2"
    elif has wget; then
        wget -q -O "$2" "$1"
    else
        fail "femp's installer needs curl or wget"
    fi
}

sha256() {
    if has sha256sum; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif has shasum; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    elif has openssl; then
        openssl dgst -sha256 "$1" | sed 's/.* //'
    else
        fail "femp's installer needs sha256sum, shasum or openssl to verify downloads"
    fi
}

# Checks `file` against the line for `name` in a SHA256SUMS-style list.
verify() {
    expected=$(awk -v name="$3" '{ file = $2; sub(/^\*/, "", file) } file == name { print $1 }' "$2")
    [ -n "$expected" ] || fail "no checksum for $3"
    actual=$(sha256 "$1")
    [ "$actual" = "$expected" ] || fail "$3 is corrupt (SHA-256 $actual, expected $expected)"
}

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        arm64 | aarch64) arch=aarch64 ;;
        *) fail "femp is not built for $arch processors" ;;
    esac
    case "$os" in
        Linux)
            if ldd --version 2>&1 | grep -qi musl; then
                fail "femp is built for glibc-based Linux; musl systems such as Alpine are not supported yet"
            fi
            target="$arch-unknown-linux-gnu"
            node_platform="linux"
            ;;
        Darwin)
            # A shell running under Rosetta reports x86_64 on Apple silicon.
            if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
                arch=aarch64
            fi
            target="$arch-apple-darwin"
            node_platform="darwin"
            ;;
        *) fail "femp runs on Linux and macOS; this system is $os" ;;
    esac
    case "$arch" in
        x86_64) node_arch=x64 ;;
        aarch64) node_arch=arm64 ;;
    esac
}

release_url() {
    if [ -n "${FEMP_DOWNLOAD_URL:-}" ]; then
        printf '%s' "${FEMP_DOWNLOAD_URL%/}"
    elif [ -n "${FEMP_VERSION:-}" ]; then
        printf 'https://github.com/%s/releases/download/%s' "$REPOSITORY" "$FEMP_VERSION"
    else
        printf 'https://github.com/%s/releases/latest/download' "$REPOSITORY"
    fi
}

install_femp() {
    archive="femp-$target.tar.gz"
    base=$(release_url)
    say "Downloading $archive"
    download "$base/$archive" "$work/$archive" || fail "could not download $base/$archive"
    download "$base/SHA256SUMS" "$work/SHA256SUMS" || fail "could not download $base/SHA256SUMS"
    verify "$work/$archive" "$work/SHA256SUMS" "$archive"
    tar -xzf "$work/$archive" -C "$work"
    unpacked="$work/femp-$target"
    [ -x "$unpacked/bin/femp" ] || fail "$archive does not contain bin/femp"

    mkdir -p "$FEMP_HOME/bin" "$FEMP_HOME/licenses"
    cp "$unpacked"/LICENSE* "$FEMP_HOME/licenses/" 2>/dev/null || true
    install_z3
    # Replaced last, so that a failed install leaves a working femp alone.
    cp "$unpacked/bin/femp" "$FEMP_HOME/bin/femp.new"
    mv "$FEMP_HOME/bin/femp.new" "$FEMP_HOME/bin/femp"

    "$FEMP_HOME/bin/femp" --version >/dev/null 2>&1 \
        || fail "the installed femp does not run on this system"
}

# The archive carries a Z3 build for this kind of system. A system too old
# for it keeps working with a `z3` on PATH, which femp falls back to.
install_z3() {
    if [ ! -d "$unpacked/z3" ]; then
        z3_status="not in this release"
        return
    fi
    rm -rf "$FEMP_HOME/z3"
    cp -R "$unpacked/z3" "$FEMP_HOME/z3"
    if bundled=$("$FEMP_HOME/z3/bin/z3" --version 2>/dev/null); then
        z3_status="$bundled ($FEMP_HOME/z3)"
    elif has z3; then
        z3_status="$(z3 --version) ($(command -v z3)); the bundled Z3 needs a newer system"
    else
        z3_status="not usable: the bundled Z3 needs a newer system; install z3 for \`femp prove\`"
    fi
}

node_major() {
    "$1" --version 2>/dev/null | sed -n 's/^v\([0-9][0-9]*\).*/\1/p'
}

node_is_new_enough() {
    major=$(node_major "$1")
    [ -n "$major" ] && [ "$major" -ge "$NODE_MIN_MAJOR" ]
}

ensure_node() {
    if has node && node_is_new_enough node; then
        node_status="$(node --version) ($(command -v node))"
        return
    fi
    if [ -x "$FEMP_HOME/node/bin/node" ] && node_is_new_enough "$FEMP_HOME/node/bin/node"; then
        node_status="$("$FEMP_HOME/node/bin/node" --version) ($FEMP_HOME/node)"
        return
    fi
    name="node-v$NODE_VERSION-$node_platform-$node_arch"
    mirror="${FEMP_NODE_MIRROR:-https://nodejs.org/dist}"
    mirror="${mirror%/}/v$NODE_VERSION"
    if has node; then
        say "Installing Node.js $NODE_VERSION for femp (the node on PATH is $(node --version), older than v$NODE_MIN_MAJOR)"
    else
        say "Installing Node.js $NODE_VERSION for femp"
    fi
    if ! download "$mirror/$name.tar.gz" "$work/$name.tar.gz" \
        || ! download "$mirror/SHASUMS256.txt" "$work/SHASUMS256.txt"; then
        warn "could not download Node.js from $mirror; WebAssembly needs Node.js $NODE_MIN_MAJOR or newer"
        node_status="not installed (download failed)"
        return
    fi
    verify "$work/$name.tar.gz" "$work/SHASUMS256.txt" "$name.tar.gz"
    tar -xzf "$work/$name.tar.gz" -C "$work"
    rm -rf "$FEMP_HOME/node"
    mv "$work/$name" "$FEMP_HOME/node"
    node_status="$("$FEMP_HOME/node/bin/node" --version) ($FEMP_HOME/node)"
}

# Native executables are linked with `cc` on x86-64 Linux and Apple silicon,
# the hosts the Fe native backend supports.
ensure_c_compiler() {
    case "$target" in
        x86_64-unknown-linux-gnu | aarch64-apple-darwin) ;;
        *)
            cc_status="not needed (native executables cannot be built on $target)"
            return
            ;;
    esac
    if has cc && cc --version >/dev/null 2>&1; then
        cc_status="found ($(command -v cc))"
        return
    fi
    case "$target" in
        *-apple-darwin) install_command_line_tools ;;
        *) install_linux_compiler ;;
    esac
}

install_command_line_tools() {
    if confirm "Native executables need Apple's Command Line Tools. Install them now?"; then
        xcode-select --install >/dev/null 2>&1 || true
        cc_status="installing: finish the Command Line Tools dialog, then run \`femp doctor\`"
    else
        cc_status="not found: run \`xcode-select --install\` to build native executables"
    fi
}

install_linux_compiler() {
    if has apt-get; then
        manager="apt-get"
        packages="gcc libc6-dev"
        command="apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $packages"
    elif has dnf; then
        manager="dnf"
        packages="gcc"
        command="dnf install -y -q $packages"
    elif has yum; then
        manager="yum"
        packages="gcc"
        command="yum install -y -q $packages"
    elif has pacman; then
        manager="pacman"
        packages="gcc"
        command="pacman -Sy --noconfirm --needed $packages"
    elif has zypper; then
        manager="zypper"
        packages="gcc"
        command="zypper --non-interactive --quiet install $packages"
    else
        cc_status="not found: install a C compiler (gcc or clang) to build native executables"
        return
    fi
    hint="sudo $manager install $packages"
    if [ "$(id -u)" = 0 ]; then
        run="sh -c"
    elif has sudo; then
        run="sudo sh -c"
    else
        cc_status="not found: as root, run \`$manager install $packages\`"
        return
    fi
    if ! confirm "Native executables need a C compiler. Install $packages with $manager?"; then
        cc_status="not found: run \`$hint\` (or rerun this installer with --yes)"
        return
    fi
    say "Installing $packages with $manager"
    if $run "$command" >/dev/null && has cc; then
        cc_status="installed ($(command -v cc))"
    else
        cc_status="not found: \`$hint\` failed"
    fi
}

add_to_profile() {
    if ! grep -qsF "$line" "$1"; then
        printf '\n%s\n' "$line" >>"$1"
    fi
}

modify_path() {
    # Startup files name the default location through $HOME, like the
    # lines other toolchain installers add.
    if [ "$FEMP_HOME" = "$HOME/.femp" ]; then
        home_text='$HOME/.femp'
    else
        home_text="$FEMP_HOME"
    fi
    {
        printf '# Added by the femp installer.\n'
        if [ "$FEMP_HOME" != "$HOME/.femp" ]; then
            printf 'export FEMP_HOME="%s"\n' "$FEMP_HOME"
        fi
        printf 'case ":${PATH}:" in\n'
        printf '    *":%s/bin:"*) ;;\n' "$home_text"
        printf '    *) export PATH="%s/bin:$PATH" ;;\n' "$home_text"
        printf 'esac\n'
    } >"$FEMP_HOME/env"
    [ "$modify_path" = true ] || return 0

    line=". \"$home_text/env\""
    add_to_profile "$HOME/.profile"
    # Bash reads these instead of .profile when they exist.
    for file in "$HOME/.bash_profile" "$HOME/.bashrc"; do
        if [ -f "$file" ]; then
            add_to_profile "$file"
        fi
    done
    case "${SHELL:-}" in
        */zsh) add_to_profile "$HOME/.zshrc" ;;
        *)
            if [ -f "$HOME/.zshrc" ]; then
                add_to_profile "$HOME/.zshrc"
            fi
            ;;
    esac
    if has fish || [ -d "$HOME/.config/fish" ]; then
        mkdir -p "$HOME/.config/fish/conf.d"
        {
            if [ "$FEMP_HOME" != "$HOME/.femp" ]; then
                printf 'set -gx FEMP_HOME "%s"\n' "$FEMP_HOME"
            fi
            printf 'fish_add_path -g "%s/bin"\n' "$FEMP_HOME"
        } >"$HOME/.config/fish/conf.d/femp.fish"
    fi
}

usage() {
    cat <<'USAGE'
Installs femp, with its Fe compiler, and Z3, and Node.js and a C compiler when missing.

Usage: installer.sh [--yes] [--no-modify-path] [--skip-deps]

  --yes             install a missing C compiler without asking
  --no-modify-path  do not add femp to PATH in shell startup files
  --skip-deps       install femp and Z3 only

Environment: FEMP_HOME, FEMP_VERSION, FEMP_DOWNLOAD_URL, FEMP_NODE_MIRROR
USAGE
}

main() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --yes | -y) yes=true ;;
            --no-modify-path) modify_path=false ;;
            --skip-deps) deps=false ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) fail "unknown option $1" ;;
        esac
        shift
    done

    [ -n "${HOME:-}" ] || fail "HOME is not set"
    FEMP_HOME="${FEMP_HOME:-$HOME/.femp}"
    detect_target
    work=$(mktemp -d 2>/dev/null || mktemp -d -t femp)
    trap 'rm -rf "$work"' EXIT
    trap 'exit 130' INT TERM

    install_femp
    node_status="not checked (--skip-deps)"
    cc_status="not checked (--skip-deps)"
    if [ "$deps" = true ]; then
        ensure_node
        ensure_c_compiler
    fi
    modify_path

    version=$("$FEMP_HOME/bin/femp" --version)
    say ""
    say "$version is installed in $FEMP_HOME"
    say "  z3    $z3_status"
    say "  node  $node_status"
    say "  cc    $cc_status"
    say ""
    case ":$PATH:" in
        *":$FEMP_HOME/bin:"*) ;;
        *)
            if [ "$modify_path" = true ]; then
                say "Open a new terminal, or run \`. \"$home_text/env\"\`, to put femp on PATH."
            else
                say "Add $home_text/bin to PATH, for example with \`. \"$home_text/env\"\`."
            fi
            ;;
    esac
    say "Then start a project:  femp new my-app && cd my-app && femp doctor"
}

main "$@"
