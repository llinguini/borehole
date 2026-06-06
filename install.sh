#!/bin/sh
# borehole CLI installer for Linux and macOS.
#
# Downloads the prebuilt `borehole` binary from the GitHub Releases of this
# repository and installs it into a directory on your PATH so it can be run as
# `borehole` instead of `./borehole-...`.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/llinguini/borehole/main/install.sh | sh
#
# Environment overrides:
#   BOREHOLE_REPO         "owner/repo" to install from (default: llinguini/borehole)
#   BOREHOLE_VERSION      tag to install, e.g. v0.1.1 (default: latest)
#   BOREHOLE_INSTALL_DIR  force the install directory (skips auto-detection)

set -eu

REPO="${BOREHOLE_REPO:-llinguini/borehole}"
VERSION="${BOREHOLE_VERSION:-latest}"
BIN_NAME="borehole"

# Abort with a message on stderr.
err() {
    echo "error: $*" >&2
    exit 1
}

info() {
    echo "$*"
}

# Map the current OS/arch to the matching Release asset name.
detect_asset() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$arch" in
        x86_64 | amd64) arch="x86_64" ;;
        aarch64 | arm64) arch="aarch64" ;;
        *) err "unsupported architecture: $arch" ;;
    esac

    case "$os" in
        Linux)
            # Only x86_64 Linux is published today (static musl build).
            [ "$arch" = "x86_64" ] \
                || err "no Linux $arch binary is published yet; build from source"
            echo "borehole-x86_64-unknown-linux-musl"
            ;;
        Darwin)
            echo "borehole-${arch}-apple-darwin"
            ;;
        *)
            err "unsupported OS: $os (use install.ps1 on Windows)"
            ;;
    esac
}

# Build the download URL for the resolved asset and version.
asset_url() {
    asset="$1"
    if [ "$VERSION" = "latest" ]; then
        echo "https://github.com/${REPO}/releases/latest/download/${asset}"
    else
        echo "https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
    fi
}

# Download $1 into the file $2 using whichever fetcher is available.
download() {
    url="$1"
    out="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fSL "$url" -o "$out"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$out" "$url"
    else
        err "neither curl nor wget is available"
    fi
}

# Resolve the install directory: an explicit override, then /usr/local/bin
# (directly, as root, or via sudo), falling back to ~/.local/bin.
#
# Echoes two space-separated tokens: the directory and the privilege wrapper
# ("sudo" or "" for none).
resolve_install_dir() {
    if [ -n "${BOREHOLE_INSTALL_DIR:-}" ]; then
        echo "$BOREHOLE_INSTALL_DIR "
        return
    fi

    system_dir="/usr/local/bin"
    if [ -w "$system_dir" ]; then
        echo "$system_dir "
    elif [ "$(id -u)" = "0" ]; then
        echo "$system_dir "
    elif command -v sudo >/dev/null 2>&1; then
        echo "$system_dir sudo"
    else
        echo "${HOME}/.local/bin "
    fi
}

main() {
    asset="$(detect_asset)"
    url="$(asset_url "$asset")"

    tmp="$(mktemp)"
    # Clean up the temp file on any exit.
    trap 'rm -f "$tmp"' EXIT INT TERM

    info "Downloading $asset ($VERSION)..."
    download "$url" "$tmp"
    chmod +x "$tmp"

    set -- $(resolve_install_dir)
    dir="$1"
    sudo_cmd="${2:-}"
    dest="${dir}/${BIN_NAME}"

    info "Installing to $dest"
    $sudo_cmd mkdir -p "$dir"
    $sudo_cmd cp "$tmp" "$dest"
    $sudo_cmd chmod +x "$dest"

    info "✓ Installed borehole to $dest"

    # Warn if the install directory is not on PATH.
    case ":${PATH}:" in
        *":${dir}:"*) ;;
        *)
            info ""
            info "⚠ $dir is not on your PATH. Add this to your shell profile:"
            info "    export PATH=\"$dir:\$PATH\""
            ;;
    esac

    info ""
    info "Run 'borehole config' to get started."
}

main
