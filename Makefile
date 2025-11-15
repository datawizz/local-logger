.PHONY: help build release test clean install pkg pkg-install pkg-uninstall init serve proxy fmt clippy

# Default target
help:
	@echo "local-logger Makefile"
	@echo ""
	@echo "Common targets:"
	@echo "  make build          - Build debug binary"
	@echo "  make release        - Build release binary"
	@echo "  make test           - Run tests"
	@echo "  make install        - Install to ~/.cargo/bin"
	@echo "  make init           - Initialize certificates"
	@echo ""
	@echo "macOS PKG targets:"
	@echo "  make pkg            - Build macOS PKG installer"
	@echo "  make pkg-install    - Build and install PKG"
	@echo "  make pkg-uninstall  - Uninstall PKG"
	@echo ""
	@echo "Development targets:"
	@echo "  make serve          - Run MCP server"
	@echo "  make proxy          - Run HTTPS proxy"
	@echo "  make fmt            - Format code"
	@echo "  make clippy         - Run clippy linter"
	@echo "  make clean          - Clean build artifacts"

# Build targets
build:
	cargo build

release:
	cargo build --release

test:
	cargo test

# Installation targets
install:
	cargo install --path .

init:
	cargo run --release -- init

# macOS PKG targets
pkg:
	./packaging/macos/build-pkg.sh

pkg-install: pkg
	sudo installer -pkg packaging/macos/build/local-logger-*.pkg -target /

pkg-uninstall:
	sudo /usr/local/bin/local-logger-uninstall.sh

# Run targets
serve:
	cargo run --release -- serve

proxy:
	cargo run --release -- proxy

# Development targets
fmt:
	cargo fmt

clippy:
	cargo clippy -- -D warnings

clean:
	cargo clean
	rm -rf packaging/macos/build
