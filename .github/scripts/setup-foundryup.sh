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

# Copy foundryup to a location in PATH
echo "Copying foundryup to /usr/local/bin..."
sudo cp foundryup/foundryup /usr/local/bin/foundryup
sudo chmod +x /usr/local/bin/foundryup

# Verify foundryup is accessible
if ! command -v foundryup &> /dev/null; then
    echo "Error: foundryup not found in PATH after installation"
    exit 1
fi

echo "foundryup is now available at: $(which foundryup)"

# Create foundry directories
echo "Creating foundry directories..."
mkdir -p "$HOME/.foundry/bin"
mkdir -p "$HOME/.foundry/versions"

# Export PATH for current session
export PATH="$HOME/.foundry/bin:$PATH"

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