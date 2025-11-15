#!/bin/bash
#
# Build macOS PKG installer for local-logger
#
# This script:
# 1. Builds the release binary
# 2. Creates a payload directory structure
# 3. Builds the PKG with pkgbuild and productbuild
# 4. Optionally signs the PKG (when Developer ID certificate is available)
#

set -e  # Exit on error
set -u  # Exit on undefined variable

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

# Build configuration
BUILD_DIR="$PROJECT_ROOT/packaging/macos/build"
PAYLOAD_DIR="$BUILD_DIR/payload"
COMPONENT_PKG="$BUILD_DIR/local-logger-component.pkg"

# Get version from Cargo.toml
VERSION=$(grep "^version = " Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
if [ -z "$VERSION" ]; then
    echo -e "${RED}✗ Failed to extract version from Cargo.toml${NC}"
    exit 1
fi

PKG_NAME="local-logger-${VERSION}.pkg"
FINAL_PKG="$BUILD_DIR/$PKG_NAME"

# Package identifier
PKG_IDENTIFIER="org.cogent-creation-co.local-logger.pkg"

# Installation paths
INSTALL_ROOT="/"
BINARY_INSTALL_PATH="usr/local/bin"
LAUNCHAGENT_INSTALL_PATH="Library/LaunchAgents"

echo -e "${BLUE}======================================${NC}"
echo -e "${BLUE}  Building local-logger PKG installer${NC}"
echo -e "${BLUE}======================================${NC}"
echo ""
echo -e "${GREEN}Version:${NC} $VERSION"
echo -e "${GREEN}Project root:${NC} $PROJECT_ROOT"
echo -e "${GREEN}Build directory:${NC} $BUILD_DIR"
echo ""

# Clean previous build
if [ -d "$BUILD_DIR" ]; then
    echo -e "${YELLOW}→ Cleaning previous build...${NC}"
    rm -rf "$BUILD_DIR"
fi

# Create build directories
echo -e "${YELLOW}→ Creating build directories...${NC}"
mkdir -p "$PAYLOAD_DIR/$BINARY_INSTALL_PATH"
mkdir -p "$PAYLOAD_DIR/$LAUNCHAGENT_INSTALL_PATH"

# Build the Rust binary
echo ""
echo -e "${YELLOW}→ Building release binary...${NC}"
cargo build --release

if [ ! -f "target/release/local-logger" ]; then
    echo -e "${RED}✗ Failed to build binary${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Binary built successfully${NC}"

# Copy binary to payload
echo -e "${YELLOW}→ Copying binary to payload...${NC}"
cp target/release/local-logger "$PAYLOAD_DIR/$BINARY_INSTALL_PATH/"
chmod 755 "$PAYLOAD_DIR/$BINARY_INSTALL_PATH/local-logger"

# Copy uninstaller to payload
echo -e "${YELLOW}→ Copying uninstaller to payload...${NC}"
cp packaging/macos/uninstall/local-logger-uninstall.sh "$PAYLOAD_DIR/$BINARY_INSTALL_PATH/"
chmod 755 "$PAYLOAD_DIR/$BINARY_INSTALL_PATH/local-logger-uninstall.sh"

# Copy LaunchAgent plist to payload
echo -e "${YELLOW}→ Copying LaunchAgent plist to payload...${NC}"
cp packaging/macos/launchd/org.cogent-creation-co.local-logger-proxy.plist "$PAYLOAD_DIR/$LAUNCHAGENT_INSTALL_PATH/"
chmod 644 "$PAYLOAD_DIR/$LAUNCHAGENT_INSTALL_PATH/org.cogent-creation-co.local-logger-proxy.plist"

# Ensure scripts are executable
echo -e "${YELLOW}→ Setting script permissions...${NC}"
chmod 755 packaging/macos/scripts/preinstall
chmod 755 packaging/macos/scripts/postinstall

# Build component package
echo ""
echo -e "${YELLOW}→ Building component package...${NC}"
pkgbuild \
    --root "$PAYLOAD_DIR" \
    --scripts "$SCRIPT_DIR/scripts" \
    --identifier "$PKG_IDENTIFIER" \
    --version "$VERSION" \
    --install-location "$INSTALL_ROOT" \
    "$COMPONENT_PKG"

echo -e "${GREEN}✓ Component package created${NC}"

# Create distribution XML for productbuild
echo -e "${YELLOW}→ Generating distribution XML...${NC}"
DISTRIBUTION_XML="$BUILD_DIR/distribution.xml"

cat > "$DISTRIBUTION_XML" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>Local Logger</title>
    <welcome file="welcome.html"/>
    <conclusion file="conclusion.html"/>
    <pkg-ref id="$PKG_IDENTIFIER"/>
    <options customize="never" require-scripts="false" hostArchitectures="arm64,x86_64"/>
    <choices-outline>
        <line choice="default">
            <line choice="$PKG_IDENTIFIER"/>
        </line>
    </choices-outline>
    <choice id="default"/>
    <choice id="$PKG_IDENTIFIER" visible="false">
        <pkg-ref id="$PKG_IDENTIFIER"/>
    </choice>
    <pkg-ref id="$PKG_IDENTIFIER" version="$VERSION" onConclusion="none">local-logger-component.pkg</pkg-ref>
</installer-gui-script>
EOF

# Build final product package
echo -e "${YELLOW}→ Building final PKG...${NC}"

# Check if signing certificate is available
SIGN_IDENTITY=""
if security find-identity -p basic -v | grep -q "Developer ID Installer"; then
    SIGN_IDENTITY=$(security find-identity -p basic -v | grep "Developer ID Installer" | head -1 | sed 's/.*"\(.*\)"/\1/')
    echo -e "${BLUE}  Found signing identity: $SIGN_IDENTITY${NC}"
    echo -e "${YELLOW}  Building signed PKG...${NC}"

    productbuild \
        --distribution "$DISTRIBUTION_XML" \
        --resources "$SCRIPT_DIR/resources" \
        --package-path "$BUILD_DIR" \
        --sign "$SIGN_IDENTITY" \
        "$FINAL_PKG"
else
    echo -e "${YELLOW}  No Developer ID Installer certificate found${NC}"
    echo -e "${YELLOW}  Building unsigned PKG...${NC}"

    productbuild \
        --distribution "$DISTRIBUTION_XML" \
        --resources "$SCRIPT_DIR/resources" \
        --package-path "$BUILD_DIR" \
        "$FINAL_PKG"
fi

# Success!
echo ""
echo -e "${GREEN}======================================${NC}"
echo -e "${GREEN}  ✓ PKG built successfully!${NC}"
echo -e "${GREEN}======================================${NC}"
echo ""
echo -e "${GREEN}Package:${NC} $FINAL_PKG"
echo -e "${GREEN}Size:${NC} $(du -h "$FINAL_PKG" | cut -f1)"
echo ""
echo -e "${BLUE}To install:${NC}"
echo -e "  sudo installer -pkg \"$FINAL_PKG\" -target /"
echo ""
echo -e "${BLUE}To test:${NC}"
echo -e "  1. Install the PKG"
echo -e "  2. Verify binary: which local-logger"
echo -e "  3. Check certificates: ls -la ~/.local-logger/certs/"
echo -e "  4. Verify LaunchAgent: launchctl list | grep local-logger"
echo ""
