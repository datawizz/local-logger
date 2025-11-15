#!/bin/bash
# Conflict detection for local-logger installations
# Prevents PKG and Nix installations from conflicting

set -e

# Colors for output
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

detect_pkg_install() {
    # Check for PKG installation (non-symlink binary in /usr/local/bin)
    [ -f "/usr/local/bin/local-logger" ] && [ ! -L "/usr/local/bin/local-logger" ]
}

detect_nix_install() {
    # Check if nix-darwin configuration includes local-logger service
    if [ -f "/etc/static/darwin-configuration.nix" ]; then
        grep -q "services.local-logger.enable = true" /etc/static/darwin-configuration.nix 2>/dev/null
        return $?
    fi
    return 1
}

# Usage: detect_and_exit_if_pkg (called from Nix activation)
detect_and_exit_if_pkg() {
    if detect_pkg_install; then
        echo -e "${RED}ERROR: local-logger PKG installation detected${NC}" >&2
        echo "" >&2
        echo "Found: /usr/local/bin/local-logger" >&2
        echo "" >&2
        echo "To use Nix-managed local-logger, first uninstall the PKG version:" >&2
        echo "  sudo /usr/local/bin/local-logger-uninstall.sh" >&2
        echo "" >&2
        return 1
    fi
    return 0
}

# Usage: detect_and_exit_if_nix (called from PKG preinstall)
detect_and_exit_if_nix() {
    if detect_nix_install; then
        echo -e "${RED}ERROR: local-logger is managed by nix-darwin${NC}" >&2
        echo "" >&2
        echo "To install via PKG, first disable in your nix configuration:" >&2
        echo "  services.local-logger.enable = false;" >&2
        echo "" >&2
        echo "Then rebuild:" >&2
        echo "  darwin-rebuild switch" >&2
        echo "" >&2
        return 1
    fi
    return 0
}

# Allow sourcing this script to use the functions
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    # Called directly, print usage
    echo "Usage:"
    echo "  Source this script and call:"
    echo "    detect_and_exit_if_pkg  # For Nix activation"
    echo "    detect_and_exit_if_nix  # For PKG preinstall"
fi
