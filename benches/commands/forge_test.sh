#!/bin/bash

# Forge Test Benchmark Command
# This file contains the configuration and execution logic for benchmarking 'forge test'

# Command configuration
FORGE_TEST_RUNS=5
FORGE_TEST_WARMUP=1

# Benchmark function for forge test
benchmark_forge_test() {
    local repo_name=$1
    local version=$2
    local version_results_dir=$3
    local log_file=$4
    
    echo "Running 'forge test' benchmark..." >> "$log_file"
    
    if hyperfine \
        --runs "$FORGE_TEST_RUNS" \
        --prepare 'forge build' \
        --warmup "$FORGE_TEST_WARMUP" \
        --export-json "${version_results_dir}/test_results.json" \
        "forge test" 2>>"$log_file.error"; then
        echo "✓ forge test completed" >> "$log_file"
        return 0
    else
        echo "✗ forge test failed" >> "$log_file"
        echo "FATAL: forge test benchmark failed" >> "$log_file"
        return 1
    fi
}

# Get command description for reporting
get_forge_test_description() {
    echo "forge test - Running the test suite ($FORGE_TEST_RUNS runs, $FORGE_TEST_WARMUP warmup)"
}

# Get JSON result filename
get_forge_test_json_filename() {
    echo "test_results.json"
}

# Get benchmark type identifier
get_forge_test_type() {
    echo "test"
}