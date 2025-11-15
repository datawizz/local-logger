#!/bin/bash
#
# Uninstaller for local-logger
#
# This script removes all local-logger components:
# 1. Stops and unloads the LaunchAgent
# 2. Removes the binary and uninstaller
# 3. Optionally removes certificates and logs
# 4. Optionally removes the CA cert from system keychain
#
# Usage: sudo ./local-logger-uninstall.sh
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo -e "${RED}This script must be run as root${NC}"
    echo "Please run: sudo $0"
    exit 1
fi

echo -e "${BLUE}======================================${NC}"
echo -e "${BLUE}  local-logger Uninstaller${NC}"
echo -e "${BLUE}======================================${NC}"
echo ""

# Determine the current user
CURRENT_USER=$(stat -f '%Su' /dev/console)
USER_HOME=$(eval echo "~$CURRENT_USER")
USER_UID=$(id -u "$CURRENT_USER")

echo -e "${GREEN}Uninstalling for user:${NC} $CURRENT_USER"
echo ""

# LaunchAgent configuration
LAUNCHAGENT_LABEL="org.local-logger.proxy"
LAUNCHAGENT_PLIST="/Library/LaunchAgents/$LAUNCHAGENT_LABEL.plist"

# Stop and unload LaunchAgent
if [ -f "$LAUNCHAGENT_PLIST" ]; then
    echo -e "${YELLOW}→ Stopping LaunchAgent...${NC}"

    if sudo -u "$CURRENT_USER" launchctl list | grep -q "$LAUNCHAGENT_LABEL" 2>/dev/null; then
        # Try modern bootout method
        if sudo -u "$CURRENT_USER" launchctl bootout "gui/$USER_UID/$LAUNCHAGENT_LABEL" 2>/dev/null; then
            echo -e "${GREEN}✓ LaunchAgent stopped${NC}"
        # Fallback to legacy unload
        elif sudo -u "$CURRENT_USER" launchctl unload "$LAUNCHAGENT_PLIST" 2>/dev/null; then
            echo -e "${GREEN}✓ LaunchAgent stopped (legacy method)${NC}"
        else
            echo -e "${YELLOW}  LaunchAgent was not running${NC}"
        fi
    else
        echo -e "${YELLOW}  LaunchAgent was not loaded${NC}"
    fi

    # Remove the plist file
    echo -e "${YELLOW}→ Removing LaunchAgent plist...${NC}"
    rm -f "$LAUNCHAGENT_PLIST"
    echo -e "${GREEN}✓ LaunchAgent plist removed${NC}"
fi

# Ask about removing Claude Code configuration (BEFORE removing the binary)
BINARY="/usr/local/bin/local-logger"
if [ -f "$BINARY" ]; then
    echo ""
    echo -e "${YELLOW}Remove local-logger from Claude Code configuration?${NC}"
    read -p "(y/N): " -n 1 -r
    echo ""

    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${YELLOW}→ Removing local-logger from Claude Code...${NC}"

        if sudo -u "$CURRENT_USER" "$BINARY" uninstall-claude 2>/dev/null; then
            echo -e "${GREEN}✓ Removed from Claude Code configuration${NC}"
        else
            echo -e "${YELLOW}  Could not remove from Claude Code${NC}"
            echo -e "${YELLOW}  Manually edit ~/.claude.json and ~/.claude/settings.json${NC}"
        fi
    fi
fi

# Remove binaries
echo -e "${YELLOW}→ Removing binaries...${NC}"

if [ -f "/usr/local/bin/local-logger" ]; then
    rm -f "/usr/local/bin/local-logger"
    echo -e "${GREEN}✓ Binary removed${NC}"
fi

if [ -f "/usr/local/bin/local-logger-uninstall.sh" ]; then
    rm -f "/usr/local/bin/local-logger-uninstall.sh"
    echo -e "${GREEN}✓ Uninstaller removed${NC}"
fi

# Ask about removing certificates and data
LOGGER_DIR="$USER_HOME/.local-logger"
CA_CERT="$LOGGER_DIR/certs/ca.pem"

if [ -d "$LOGGER_DIR" ]; then
    echo ""
    echo -e "${YELLOW}The following data directory exists:${NC}"
    echo -e "  $LOGGER_DIR"
    echo ""
    echo -e "${YELLOW}This contains:${NC}"
    [ -d "$LOGGER_DIR/certs" ] && echo -e "  - Certificates"
    [ -f "$LOGGER_DIR"/*.jsonl ] 2>/dev/null && echo -e "  - Log files"
    echo ""
    read -p "Do you want to remove this directory? (y/N): " -n 1 -r
    echo ""

    if [[ $REPLY =~ ^[Yy]$ ]]; then
        # Ask about removing CA cert from keychain first
        if [ -f "$CA_CERT" ]; then
            echo ""
            read -p "Remove CA certificate from system keychain? (y/N): " -n 1 -r
            echo ""

            if [[ $REPLY =~ ^[Yy]$ ]]; then
                echo -e "${YELLOW}→ Removing CA certificate from system keychain...${NC}"

                # Find and remove the certificate
                # This is tricky because we need to identify it by its content
                CERT_SHA=$(security find-certificate -a -Z /Library/Keychains/System.keychain | \
                    grep -B 3 "Local Logger CA" | grep "SHA-1 hash:" | head -1 | awk '{print $3}')

                if [ -n "$CERT_SHA" ]; then
                    if security delete-certificate -Z "$CERT_SHA" /Library/Keychains/System.keychain 2>/dev/null; then
                        echo -e "${GREEN}✓ CA certificate removed from system keychain${NC}"
                    else
                        echo -e "${YELLOW}  Could not remove CA certificate automatically${NC}"
                        echo -e "${YELLOW}  You may need to remove it manually via Keychain Access${NC}"
                    fi
                else
                    echo -e "${YELLOW}  Could not find CA certificate in system keychain${NC}"
                fi
            fi
        fi

        # Remove the data directory
        echo -e "${YELLOW}→ Removing data directory...${NC}"
        rm -rf "$LOGGER_DIR"
        echo -e "${GREEN}✓ Data directory removed${NC}"
    else
        echo -e "${BLUE}  Keeping data directory${NC}"
        echo -e "${BLUE}  Location: $LOGGER_DIR${NC}"
    fi
fi

echo ""
echo -e "${GREEN}======================================${NC}"
echo -e "${GREEN}  ✓ Uninstallation complete${NC}"
echo -e "${GREEN}======================================${NC}"
echo ""
echo -e "${GREEN}local-logger has been removed from your system${NC}"
echo ""

exit 0
