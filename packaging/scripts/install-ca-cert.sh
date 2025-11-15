#!/bin/bash
#
# Helper script to install local-logger CA certificate to system keychain
#
# This script can be run standalone if the PKG installation didn't
# automatically trust the certificate, or if you need to reinstall it.
#
# Usage: sudo ./install-ca-cert.sh [path-to-ca.pem]
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Check if running as root
if [ "$EUID" -ne 0" ]; then
    echo -e "${RED}This script must be run as root${NC}"
    echo "Please run: sudo $0"
    exit 1
fi

# Determine CA certificate path
if [ -n "$1" ]; then
    CA_CERT="$1"
else
    # Default location
    CURRENT_USER=$(stat -f '%Su' /dev/console)
    USER_HOME=$(eval echo "~$CURRENT_USER")
    CA_CERT="$USER_HOME/.local-logger/certs/ca.pem"
fi

echo -e "${BLUE}======================================${NC}"
echo -e "${BLUE}  CA Certificate Installer${NC}"
echo -e "${BLUE}======================================${NC}"
echo ""
echo -e "${GREEN}CA Certificate:${NC} $CA_CERT"
echo ""

# Verify certificate exists
if [ ! -f "$CA_CERT" ]; then
    echo -e "${RED}✗ Certificate not found at: $CA_CERT${NC}"
    echo ""
    echo "Please provide the path to your CA certificate:"
    echo "  sudo $0 /path/to/ca.pem"
    echo ""
    echo "Or generate it first with:"
    echo "  local-logger init"
    exit 1
fi

echo -e "${GREEN}✓ Certificate found${NC}"

# Check if already trusted
echo -e "${YELLOW}→ Checking if certificate is already trusted...${NC}"

if security verify-cert -c "$CA_CERT" >/dev/null 2>&1; then
    echo -e "${GREEN}✓ Certificate is already trusted in system keychain${NC}"
    echo ""
    echo "Nothing to do!"
    exit 0
fi

echo -e "${YELLOW}  Certificate is not yet trusted${NC}"

# Install to system keychain
echo -e "${YELLOW}→ Installing certificate to system keychain...${NC}"

if security add-trusted-cert -d -r trustRoot \
    -k /Library/Keychains/System.keychain \
    "$CA_CERT" 2>/dev/null; then
    echo -e "${GREEN}✓ Certificate installed and trusted successfully${NC}"
else
    echo -e "${RED}✗ Failed to install certificate${NC}"
    echo ""
    echo "You can try manually:"
    echo "  1. Open Keychain Access"
    echo "  2. Select 'System' keychain"
    echo "  3. Drag and drop: $CA_CERT"
    echo "  4. Double-click the certificate"
    echo "  5. Expand 'Trust' section"
    echo "  6. Set 'When using this certificate' to 'Always Trust'"
    exit 1
fi

# Verify it worked
echo -e "${YELLOW}→ Verifying installation...${NC}"

if security verify-cert -c "$CA_CERT" >/dev/null 2>&1; then
    echo -e "${GREEN}✓ Certificate verification successful${NC}"
else
    echo -e "${YELLOW}  Certificate installed but verification failed${NC}"
    echo -e "${YELLOW}  You may need to manually set trust settings in Keychain Access${NC}"
fi

echo ""
echo -e "${GREEN}======================================${NC}"
echo -e "${GREEN}  ✓ Installation complete${NC}"
echo -e "${GREEN}======================================${NC}"
echo ""
echo "The local-logger CA certificate is now trusted."
echo "HTTPS interception will work for Claude API traffic."
echo ""

exit 0
