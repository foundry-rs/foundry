#!/bin/bash
set -euo pipefail

# Script to install all dependencies needed for benchmarking
# This includes Node.js/npm (via nvm) and hyperfine

echo "Installing benchmark dependencies..."

# Install Node.js and npm using nvm
echo "=== Installing Node.js and npm ==="

# Download and install nvm
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash

# Load nvm without restarting the shell
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"

# Download and install Node.js
echo "Installing Node.js 24..."
nvm install 24

# Use Node.js 24
nvm use 24

# Verify the Node.js version
echo "Node.js version: $(node -v)"
echo "Current nvm version: $(nvm current)"

# Verify npm version
echo "npm version: $(npm -v)"

# Export PATH for the current session
export PATH="$HOME/.nvm/versions/node/v24.3.0/bin:$PATH"

echo "Node.js path: $(which node)"
echo "npm path: $(which npm)"

# Install hyperfine
echo ""
echo "=== Installing hyperfine ==="

# Download and extract hyperfine binary
curl -L https://github.com/sharkdp/hyperfine/releases/download/v1.19.0/hyperfine-v1.19.0-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv hyperfine-v1.19.0-x86_64-unknown-linux-gnu/hyperfine /usr/local/bin/
rm -rf hyperfine-v1.19.0-x86_64-unknown-linux-gnu

# Verify hyperfine installation
echo "hyperfine version: $(hyperfine --version)"
echo "hyperfine path: $(which hyperfine)"

echo ""
echo "All dependencies installed successfully!"