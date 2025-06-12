#!/bin/bash

# Forge Build (With Cache) Benchmark Command
# This file contains the configuration and execution logic for benchmarking 'forge build' with cache

# Command configuration
FORGE_BUILD_WITH_CACHE_RUNS=5
FORGE_BUILD_WITH_CACHE_WARMUP=1

# Benchmark function for forge build (with cache)
benchmark_forge_build_with_cache() {
    local repo_name=$1
    local version=$2
    local version_results_dir=$3
    local log_file=$4
    
    echo "Running 'forge build' (with cache) benchmark..." >> "$log_file"
    
    if hyperfine \
        --runs "$FORGE_BUILD_WITH_CACHE_RUNS" \
        --prepare 'forge build' \
        --warmup "$FORGE_BUILD_WITH_CACHE_WARMUP" \
        --export-json "${version_results_dir}/build_with_cache_results.json" \
        "forge build" 2>>"$log_file.error"; then
        echo "✓ forge build (with cache) completed" >> "$log_file"
        return 0
    else
        echo "✗ forge build (with cache) failed" >> "$log_file"
        echo "FATAL: forge build (with cache) benchmark failed" >> "$log_file"
        return 1
    fi
}

# Get command description for reporting
get_forge_build_with_cache_description() {
    echo "forge build (with cache) - Build with warm cache ($FORGE_BUILD_WITH_CACHE_RUNS runs, $FORGE_BUILD_WITH_CACHE_WARMUP warmup)"
}

# Get JSON result filename
get_forge_build_with_cache_json_filename() {
    echo "build_with_cache_results.json"
}

# Get benchmark type identifier
get_forge_build_with_cache_type() {
    echo "build_with_cache"
}