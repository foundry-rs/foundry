#!/bin/bash

# Forge Build (No Cache) Benchmark Command
# This file contains the configuration and execution logic for benchmarking 'forge build' with no cache

# Command configuration
FORGE_BUILD_NO_CACHE_RUNS=5

# Benchmark function for forge build (no cache)
benchmark_forge_build_no_cache() {
    local repo_name=$1
    local version=$2
    local version_results_dir=$3
    local log_file=$4
    
    echo "Running 'forge build' (no cache) benchmark..." >> "$log_file"
    
    if hyperfine \
        --runs "$FORGE_BUILD_NO_CACHE_RUNS" \
        --prepare 'forge clean' \
        --export-json "${version_results_dir}/build_no_cache_results.json" \
        "forge build" 2>>"$log_file.error"; then
        echo "✓ forge build (no cache) completed" >> "$log_file"
        return 0
    else
        echo "✗ forge build (no cache) failed" >> "$log_file"
        echo "FATAL: forge build (no cache) benchmark failed" >> "$log_file"
        return 1
    fi
}

# Get command description for reporting
get_forge_build_no_cache_description() {
    echo "forge build (no cache) - Clean build without cache ($FORGE_BUILD_NO_CACHE_RUNS runs, cache cleaned after each run)"
}

# Get JSON result filename
get_forge_build_no_cache_json_filename() {
    echo "build_no_cache_results.json"
}

# Get benchmark type identifier
get_forge_build_no_cache_type() {
    echo "build_no_cache"
}