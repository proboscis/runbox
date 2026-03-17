.PHONY: build install test clean fmt lint check all

# Default target
all: build

# Build release binaries
build:
	cargo build --locked --release -p runbox-cli -p runbox-daemon

# Build only CLI (faster, skips daemon)
build-cli:
	cargo build --locked --release -p runbox-cli

# Install to ~/.cargo/bin
install: build
	cp target/release/runbox ~/.cargo/bin/runbox
	cp target/release/runbox-daemon ~/.cargo/bin/runbox-daemon
	@echo "Installed runbox and runbox-daemon to ~/.cargo/bin"

# Install only CLI
install-cli: build-cli
	cp target/release/runbox ~/.cargo/bin/runbox
	@echo "Installed runbox to ~/.cargo/bin"

# Run all tests
test:
	cargo test --locked --workspace

# Run tests with output
test-verbose:
	cargo test --locked --workspace -- --nocapture

# Format code
fmt:
	cargo fmt --all

# Check formatting
fmt-check:
	cargo fmt --all -- --check

# Run clippy
lint:
	cargo clippy --locked --workspace -- -D warnings

# Check (compile without codegen)
check:
	cargo check --locked --workspace

# Clean build artifacts
clean:
	cargo clean

# Full CI check: fmt, lint, test
ci: fmt-check lint test
