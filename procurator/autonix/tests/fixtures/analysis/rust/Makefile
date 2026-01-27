.PHONY: all build test lint format check clean install run dev

# Default target
all: build

# Build the application
build:
	cargo build --release

# Run tests
test:
	cargo test --all-features

# Run linter
lint:
	cargo clippy -- -D warnings

# Format code
format:
	cargo fmt --all

# Check formatting without modifying
check:
	cargo fmt --all -- --check
	cargo clippy -- -D warnings

# Clean build artifacts
clean:
	cargo clean

# Install dependencies (system packages)
install:
	# Debian/Ubuntu
	apt-get update && apt-get install -y libpq-dev libssl-dev pkg-config

# Run the application
run:
	cargo run

# Development mode with auto-reload
dev:
	cargo watch -x run
