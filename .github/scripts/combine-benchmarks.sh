#!/bin/bash
set -euo pipefail

# Script to combine individual benchmark results into LATEST.md
# Usage: ./combine-benchmarks.sh <output_dir>

OUTPUT_DIR="${1:-benches}"

# Create output directory if it doesn't exist
mkdir -p "$OUTPUT_DIR"

# Start building LATEST.md
cat > "$OUTPUT_DIR/LATEST.md" << EOF
# Foundry Benchmark Results

Generated at: $(date -u '+%Y-%m-%d %H:%M:%S UTC')

EOF

# Define the benchmark files to combine in order
BENCHMARK_FILES=(
    "forge_test_bench.md"
    "forge_build_bench.md" 
    "forge_coverage_bench.md"
)

# Add each benchmark result if it exists
for bench_file in "${BENCHMARK_FILES[@]}"; do
    if [ -f "$OUTPUT_DIR/$bench_file" ]; then
        echo "Adding $bench_file to combined results..."
        # Skip the header from individual files (first line) and append content
        tail -n +2 "$OUTPUT_DIR/$bench_file" >> "$OUTPUT_DIR/LATEST.md"
        echo "" >> "$OUTPUT_DIR/LATEST.md"
    else
        echo "Warning: $bench_file not found, skipping..."
    fi
done

echo "Successfully combined benchmark results into $OUTPUT_DIR/LATEST.md"