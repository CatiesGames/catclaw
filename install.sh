#!/bin/sh
# CatClaw installer — POSIX sh compatible
# Usage: curl -fsSL https://raw.githubusercontent.com/CatiesGames/catclaw/main/install.sh | sh

set -e

REPO="CatiesGames/catclaw"
INSTALL_DIR="$HOME/.local/bin"
BINARY="catclaw"

# Colors (if terminal supports it)
if [ -t 1 ]; then
    BOLD='\033[1m'
    GREEN='\033[32m'
    YELLOW='\033[33m'
    RED='\033[31m'
    RESET='\033[0m'
else
    BOLD='' GREEN='' YELLOW='' RED='' RESET=''
fi

info()  { printf "${GREEN}▸${RESET} %s\n" "$1"; }
warn()  { printf "${YELLOW}▸${RESET} %s\n" "$1"; }
error() { printf "${RED}✗${RESET} %s\n" "$1" >&2; exit 1; }

# Detect OS
OS=$(uname -s)
case "$OS" in
    Darwin) OS_NAME="darwin" ;;
    Linux)  OS_NAME="linux" ;;
    *)      error "Unsupported OS: $OS" ;;
esac

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
    arm64|aarch64) ARCH_NAME="arm64" ;;
    x86_64)        ARCH_NAME="x86_64" ;;
    *)             error "Unsupported architecture: $ARCH" ;;
esac

ASSET_NAME="catclaw-${OS_NAME}-${ARCH_NAME}"

info "Detected platform: ${OS_NAME}/${ARCH_NAME}"

# Fetch latest release tag
info "Checking latest release..."
RELEASE_JSON=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    -H "Accept: application/vnd.github+json") || error "Failed to fetch release info"

TAG=$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*: *"\(.*\)".*/\1/')
[ -z "$TAG" ] && error "Could not determine latest version"
info "Latest version: ${TAG}"

# Download binary
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET_NAME}"
CHECKSUMS_URL="https://github.com/${REPO}/releases/download/${TAG}/checksums.txt"

mkdir -p "$INSTALL_DIR"
TEMP_FILE=$(mktemp)
trap 'rm -f "$TEMP_FILE" "$TEMP_FILE.checksums"' EXIT

info "Downloading ${ASSET_NAME}..."
curl -fsSL -o "$TEMP_FILE" "$DOWNLOAD_URL" || error "Download failed"

# Verify checksum
info "Verifying checksum..."
curl -fsSL -o "$TEMP_FILE.checksums" "$CHECKSUMS_URL" 2>/dev/null
if [ -f "$TEMP_FILE.checksums" ]; then
    EXPECTED=$(grep "$ASSET_NAME" "$TEMP_FILE.checksums" | awk '{print $1}')
    if [ -n "$EXPECTED" ]; then
        if command -v sha256sum >/dev/null 2>&1; then
            ACTUAL=$(sha256sum "$TEMP_FILE" | awk '{print $1}')
        elif command -v shasum >/dev/null 2>&1; then
            ACTUAL=$(shasum -a 256 "$TEMP_FILE" | awk '{print $1}')
        else
            warn "No sha256 tool found — skipping verification"
            ACTUAL="$EXPECTED"
        fi

        if [ "$ACTUAL" != "$EXPECTED" ]; then
            error "Checksum mismatch! Expected: $EXPECTED Got: $ACTUAL"
        fi
        info "Checksum verified"
    else
        warn "Asset not found in checksums.txt — skipping verification"
    fi
else
    warn "checksums.txt not available — skipping verification"
fi

# Install
mv "$TEMP_FILE" "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"

# macOS: remove quarantine attribute
if [ "$OS_NAME" = "darwin" ]; then
    xattr -d com.apple.quarantine "${INSTALL_DIR}/${BINARY}" 2>/dev/null || true
fi

info "Installed to ${INSTALL_DIR}/${BINARY}"

# Check PATH
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*)
        # Already in PATH
        ;;
    *)
        warn "${INSTALL_DIR} is not in your PATH"
        SHELL_NAME=$(basename "$SHELL" 2>/dev/null || echo "sh")
        case "$SHELL_NAME" in
            bash)
                warn "Add it with: echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
                ;;
            zsh)
                warn "Add it with: echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
                ;;
            fish)
                warn "Add it with: fish_add_path ~/.local/bin"
                ;;
            *)
                warn "Add ${INSTALL_DIR} to your shell's PATH"
                ;;
        esac
        ;;
esac

printf "\n${BOLD}${GREEN}✓ CatClaw ${TAG} installed successfully!${RESET}\n\n"
printf "  Get started:\n"
printf "    ${BOLD}catclaw onboard${RESET}\n\n"
