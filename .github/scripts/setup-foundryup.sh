#!/bin/bash
set -euo pipefail

# Script to setup foundryup in CI environment
# This ensures foundryup is available in PATH for the benchmark binary

echo "Setting up foundryup..."

# Check if foundryup script exists in the repo
if [ ! -f "foundryup/foundryup" ]; then
    echo "Error: foundryup/foundryup script not found in repository"
    exit 1
fi

# Use FOUNDRY_DIR if set, otherwise default to $HOME/.foundry
export FOUNDRY_DIR="${FOUNDRY_DIR:-$HOME/.foundry}"
echo "Using FOUNDRY_DIR: $FOUNDRY_DIR"

# Create all necessary directories
mkdir -p "$FOUNDRY_DIR/bin"
mkdir -p "$FOUNDRY_DIR/versions"
mkdir -p "$FOUNDRY_DIR/share/man/man1"

# Copy foundryup to a user-writable location so it can replace itself with the Rust binary.
echo "Copying foundryup to $FOUNDRY_DIR/bin..."
cp foundryup/foundryup "$FOUNDRY_DIR/bin/foundryup"
chmod +x "$FOUNDRY_DIR/bin/foundryup"

# Export PATH for current session
export PATH="$FOUNDRY_DIR/bin:$PATH"

# Verify foundryup is accessible.
if ! command -v foundryup &> /dev/null; then
    echo "Error: foundryup not found in PATH after installation"
    exit 1
fi

echo "foundryup is now available at: $(command -v foundryup)"

# Run foundryup to install default version
echo "Installing default foundry version..."
foundryup

# Verify installation
if command -v forge &> /dev/null; then
    echo "Forge installed successfully: $(forge --version)"
else
    echo "Warning: forge not found in PATH after installation"
fi

echo "Foundry setup complete!"
