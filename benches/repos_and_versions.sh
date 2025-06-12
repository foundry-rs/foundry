#!/bin/bash

# Foundry Multi-Version Benchmarking Configuration
# This file contains the configuration for repositories and Foundry versions to benchmark

# Foundry versions to benchmark
# Supported formats:
#   - stable, nightly (special tags)
#   - v1.0.0, v1.1.0, etc. (specific versions)
#   - nightly-<commit-hash> (specific nightly builds)
#   - Any format supported by foundryup
FOUNDRY_VERSIONS=(
    "stable"
    "nightly-ac0411d0e3b9632247c9aea9535472eda09a57ae"
    "nightly"
)

# Repository configurations
# Add new repositories by adding entries to both arrays
REPO_NAMES=(
    "ithacaxyz-account"
    # "v4-core"
    "solady"
    # "morpho-blue"
    # "spark-psm"
)

REPO_URLS=(
    "https://github.com/ithacaxyz/account"
    # "https://github.com/Uniswap/v4-core"
    "https://github.com/Vectorized/solady"
    # "https://github.com/morpho-org/morpho-blue"
    # "https://github.com/sparkdotfi/spark-psm"
)

# Verify arrays have the same length
if [ ${#REPO_NAMES[@]} -ne ${#REPO_URLS[@]} ]; then
    echo "ERROR: REPO_NAMES and REPO_URLS arrays must have the same length"
    exit 1
fi

# Export variables for use in other scripts
export FOUNDRY_VERSIONS
export REPO_NAMES
export REPO_URLS