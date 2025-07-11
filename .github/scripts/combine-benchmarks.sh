#!/bin/bash
set -euo pipefail

# Script to combine individual benchmark results into LATEST.md
# Usage: ./combine-benchmarks.sh <output_dir>

OUTPUT_DIR="${1:-benches}"

# Create output directory if it doesn't exist
mkdir -p "$OUTPUT_DIR"

# Define the benchmark files and their section names
declare -A BENCHMARK_FILES=(
    ["forge_test_bench.md"]="Forge Test"
    ["forge_build_bench.md"]="Forge Build" 
    ["forge_coverage_bench.md"]="Forge Coverage"
)

# Function to extract a specific section from a benchmark file
extract_section() {
    local file=$1
    local section=$2
    local in_section=0
    
    while IFS= read -r line; do
        if [[ "$line" =~ ^##[[:space:]]+"$section" ]]; then
            in_section=1
            echo "$line"
        elif [[ $in_section -eq 1 && "$line" =~ ^##[[:space:]] && ! "$line" =~ ^##[[:space:]]+"$section" ]]; then
            break
        elif [[ $in_section -eq 1 ]]; then
            echo "$line"
        fi
    done < "$file"
}

# Function to extract summary info (repos and versions) from a file
extract_summary_info() {
    local file=$1
    local in_summary=0
    local in_repos=0
    local in_versions=0
    
    while IFS= read -r line; do
        # Check for Summary section
        if [[ "$line" =~ ^##[[:space:]]+Summary ]]; then
            in_summary=1
            continue
        fi
        
        # Check for Repositories Tested subsection
        if [[ $in_summary -eq 1 && "$line" =~ ^###[[:space:]]+Repositories[[:space:]]+Tested ]]; then
            in_repos=1
            echo "### Repositories Tested"
            echo
            continue
        fi
        
        # Check for Foundry Versions subsection
        if [[ $in_summary -eq 1 && "$line" =~ ^###[[:space:]]+Foundry[[:space:]]+Versions ]]; then
            in_repos=0
            in_versions=1
            echo "### Foundry Versions"
            echo
            continue
        fi
        
        # End of summary section
        if [[ $in_summary -eq 1 && "$line" =~ ^##[[:space:]] && ! "$line" =~ ^##[[:space:]]+Summary ]]; then
            break
        fi
        
        # Output repo or version lines
        if [[ ($in_repos -eq 1 || $in_versions -eq 1) && -n "$line" ]]; then
            echo "$line"
        fi
    done < "$file"
}

# Function to extract benchmark table from a section
extract_benchmark_table() {
    local file=$1
    local section=$2
    local in_section=0
    local found_table=0
    
    while IFS= read -r line; do
        if [[ "$line" =~ ^##[[:space:]]+"$section" ]]; then
            in_section=1
            continue
        elif [[ $in_section -eq 1 && "$line" =~ ^##[[:space:]] && ! "$line" =~ ^##[[:space:]]+"$section" ]]; then
            break
        elif [[ $in_section -eq 1 ]]; then
            # Skip empty lines before table
            if [[ -z "$line" && $found_table -eq 0 ]]; then
                continue
            fi
            # Detect table start
            if [[ "$line" =~ ^\|[[:space:]]*Repository ]]; then
                found_table=1
            fi
            # Output table lines
            if [[ $found_table -eq 1 && -n "$line" ]]; then
                echo "$line"
            fi
        fi
    done < "$file"
}

# Function to extract system information
extract_system_info() {
    local file=$1
    awk '/^## System Information/,/^$/ { if (!/^## System Information/ && !/^$/) print }' "$file"
}

# Start building LATEST.md
cat > "$OUTPUT_DIR/LATEST.md" << EOF
# 📊 Foundry Benchmark Results

**Generated at**: $(date -u '+%Y-%m-%d %H:%M:%S UTC')

EOF

# Process each benchmark file
FIRST_FILE=1
SYSTEM_INFO=""

for bench_file in "forge_test_bench.md" "forge_build_bench.md" "forge_coverage_bench.md"; do
    if [ -f "$OUTPUT_DIR/$bench_file" ]; then
        echo "Processing $bench_file..."
        
        # Get the section name
        case "$bench_file" in
            "forge_test_bench.md")
                SECTION_NAME="Forge Test"
                ;;
            "forge_build_bench.md")
                SECTION_NAME="Forge Build"
                ;;
            "forge_coverage_bench.md")
                SECTION_NAME="Forge Coverage"
                ;;
        esac
        
        # Add section header
        echo "## $SECTION_NAME" >> "$OUTPUT_DIR/LATEST.md"
        echo >> "$OUTPUT_DIR/LATEST.md"
        
        # Add summary info (repos and versions)
        extract_summary_info "$OUTPUT_DIR/$bench_file" >> "$OUTPUT_DIR/LATEST.md"
        echo >> "$OUTPUT_DIR/LATEST.md"
        
        # For build benchmarks, add both sub-sections
        if [[ "$bench_file" == "forge_build_bench.md" ]]; then
            # Extract No Cache table
            echo "### No Cache" >> "$OUTPUT_DIR/LATEST.md"
            echo >> "$OUTPUT_DIR/LATEST.md"
            extract_benchmark_table "$OUTPUT_DIR/$bench_file" "Forge Build (No Cache)" >> "$OUTPUT_DIR/LATEST.md"
            echo >> "$OUTPUT_DIR/LATEST.md"
            
            # Extract With Cache table
            echo "### With Cache" >> "$OUTPUT_DIR/LATEST.md"
            echo >> "$OUTPUT_DIR/LATEST.md"
            extract_benchmark_table "$OUTPUT_DIR/$bench_file" "Forge Build (With Cache)" >> "$OUTPUT_DIR/LATEST.md"
        else
            # Extract the benchmark table for other types
            extract_benchmark_table "$OUTPUT_DIR/$bench_file" "$SECTION_NAME" >> "$OUTPUT_DIR/LATEST.md"
        fi
        
        echo >> "$OUTPUT_DIR/LATEST.md"
        
        # Extract system info from first file only
        if [[ $FIRST_FILE -eq 1 ]]; then
            SYSTEM_INFO=$(extract_system_info "$OUTPUT_DIR/$bench_file")
            FIRST_FILE=0
        fi
    else
        echo "Warning: $bench_file not found, skipping..."
    fi
done

# Add system information at the end
if [[ -n "$SYSTEM_INFO" ]]; then
    echo "## System Information" >> "$OUTPUT_DIR/LATEST.md"
    echo >> "$OUTPUT_DIR/LATEST.md"
    echo "$SYSTEM_INFO" >> "$OUTPUT_DIR/LATEST.md"
fi

echo "Successfully combined benchmark results into $OUTPUT_DIR/LATEST.md"