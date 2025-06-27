#!/bin/bash

# Foundry Benchmark Runner with Criterion Table Output
# This script runs the criterion-based benchmarks and generates a markdown report

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Check if required tools are installed
check_dependencies() {
    if ! command -v criterion-table &> /dev/null; then
        log_error "criterion-table is not installed. Please install it with:"
        echo "cargo install criterion-table"
        exit 1
    fi
    
    if ! cargo criterion --help &> /dev/null; then
        log_error "cargo-criterion is not installed. Please install it with:"
        echo "cargo install cargo-criterion"
        exit 1
    fi
}

# Install Foundry versions if requested
install_foundry_versions() {
    if [[ "$FORCE_INSTALL" != "true" ]]; then
        return
    fi
    
    local versions
    
    # Use custom versions if provided, otherwise read from lib.rs
    if [[ -n "$CUSTOM_VERSIONS" ]]; then
        versions=$(echo "$CUSTOM_VERSIONS" | tr ',' ' ')
        log_info "Installing custom Foundry versions: $versions"
    else
        # Read the versions from the Rust source
        versions=$(grep -A 10 'pub static FOUNDRY_VERSIONS' src/lib.rs | grep -o '"[^"]*"' | tr -d '"')
        log_info "Installing default Foundry versions from lib.rs: $versions"
    fi
    
    # Check if foundryup is available
    if ! command -v foundryup &> /dev/null; then
        log_error "foundryup not found. Please install Foundry first:"
        echo "curl -L https://foundry.paradigm.xyz | bash"
        exit 1
    fi
    
    # Install each version
    for version in $versions; do
        log_info "Installing Foundry version: $version"
        if foundryup --install "$version"; then
            log_success "✓ Successfully installed version $version"
        else
            log_error "Failed to install Foundry version: $version"
            exit 1
        fi
    done
    
    log_success "All Foundry versions installed successfully"
}

# Get system information
get_system_info() {
    local os_name=$(uname -s)
    local arch=$(uname -m)
    local date=$(date)
    
    echo "- **OS:** $os_name"
    echo "- **Architecture:** $arch"
    echo "- **Date:** $date"
}


# Run benchmarks and generate report
run_benchmarks() {
    log_info "Running Foundry benchmarks..."
    
    # Set environment variable for custom versions if provided
    if [[ -n "$CUSTOM_VERSIONS" ]]; then
        export FOUNDRY_BENCH_VERSIONS="$CUSTOM_VERSIONS"
        log_info "Set FOUNDRY_BENCH_VERSIONS=$CUSTOM_VERSIONS"
    fi
    
    # Create temp files for each benchmark
    local temp_dir=$(mktemp -d)
    local forge_test_json="$temp_dir/forge_test.json"
    local forge_build_no_cache_json="$temp_dir/forge_build_no_cache.json"
    local forge_build_with_cache_json="$temp_dir/forge_build_with_cache.json"
    
    # Set up output redirection based on verbose flag
    local output_redirect=""
    if [[ "${VERBOSE:-false}" != "true" ]]; then
        output_redirect="2>/dev/null"
    fi
    
    # Run benchmarks in specific order (this determines baseline column)
    log_info "Running forge_test benchmark..."
    if [[ "${VERBOSE:-false}" == "true" ]]; then
        cargo criterion --bench forge_test --message-format=json > "$forge_test_json" || {
            log_error "forge_test benchmark failed"
            exit 1
        }
    else
        cargo criterion --bench forge_test --message-format=json > "$forge_test_json" 2>/dev/null || {
            log_error "forge_test benchmark failed"
            exit 1
        }
    fi
    
    log_info "Running forge_build_no_cache benchmark..."
    if [[ "${VERBOSE:-false}" == "true" ]]; then
        cargo criterion --bench forge_build_no_cache --message-format=json > "$forge_build_no_cache_json" || {
            log_error "forge_build_no_cache benchmark failed"
            exit 1
        }
    else
        cargo criterion --bench forge_build_no_cache --message-format=json > "$forge_build_no_cache_json" 2>/dev/null || {
            log_error "forge_build_no_cache benchmark failed"
            exit 1
        }
    fi
    
    log_info "Running forge_build_with_cache benchmark..."
    if [[ "${VERBOSE:-false}" == "true" ]]; then
        cargo criterion --bench forge_build_with_cache --message-format=json > "$forge_build_with_cache_json" || {
            log_error "forge_build_with_cache benchmark failed"
            exit 1
        }
    else
        cargo criterion --bench forge_build_with_cache --message-format=json > "$forge_build_with_cache_json" 2>/dev/null || {
            log_error "forge_build_with_cache benchmark failed"
            exit 1
        }
    fi
    
    # Combine all results and generate markdown
    log_info "Generating markdown report with criterion-table..."
    
    if ! cat "$forge_test_json" "$forge_build_no_cache_json" "$forge_build_with_cache_json" | criterion-table > "$temp_dir/tables.md"; then
        log_error "criterion-table failed to process benchmark data"
        exit 1
    fi
        
    # Generate the final report
    generate_report "$temp_dir/tables.md"
    
    # Cleanup
    rm -rf "$temp_dir"
    
    log_success "Benchmark report generated in LATEST.md"
}

# Generate the final markdown report
generate_report() {
    local tables_file="$1"
    local report_file="LATEST.md"
    
    log_info "Generating final report..."
    
    # Get current timestamp
    local timestamp=$(date)
    
    # Get repository information and create numbered list with links
    local versions
    if [[ -n "$CUSTOM_VERSIONS" ]]; then
        versions=$(echo "$CUSTOM_VERSIONS" | tr ',' ' ')
    else
        versions=$(grep -A 10 'pub static FOUNDRY_VERSIONS' src/lib.rs | grep -o '"[^"]*"' | tr -d '"' | tr '\n' ' ')
    fi
    
    # Extract repository info for numbered list
    local repo_list=""
    local counter=1
    
    # Parse the BENCHMARK_REPOS section
    while IFS= read -r line; do
        if [[ $line =~ RepoConfig.*name:.*\"([^\"]+)\".*org:.*\"([^\"]+)\".*repo:.*\"([^\"]+)\" ]]; then
            local name="${BASH_REMATCH[1]}"
            local org="${BASH_REMATCH[2]}"
            local repo="${BASH_REMATCH[3]}"
            repo_list+="$counter. [$name](https://github.com/$org/$repo)\n"
            ((counter++))
        fi
    done < <(grep -A 20 'pub static BENCHMARK_REPOS' src/lib.rs | grep 'RepoConfig')
    
    # Write the report
    cat > "$report_file" << EOF
# Foundry Benchmarking Results

**Generated on:** $timestamp  
**Foundry Versions Tested:** $versions  

## Repositories Tested

$(echo -e "$repo_list")

## Summary

This report contains comprehensive benchmarking results comparing different Foundry versions across multiple projects using Criterion.rs for precise performance measurements.

The following benchmarks were performed:

1. **forge-test** - Running the test suite (10 samples each)
2. **forge-build-no-cache** - Clean build without cache (10 samples each)  
3. **forge-build-with-cache** - Build with warm cache (10 samples each)

---

EOF

    # Append the criterion-table generated tables
    cat "$tables_file" >> "$report_file"
    
    # Add notes and system info
    cat >> "$report_file" << EOF
## Notes

- All benchmarks use Criterion.rs for statistical analysis
- Each benchmark runs 10 samples by default
- Results show mean execution time with confidence intervals
- Repositories are cloned once and reused across all Foundry versions
- Build and setup operations are parallelized using Rayon
- The first version tested becomes the baseline for comparisons

## System Information

$(get_system_info)

## Raw Data

Detailed benchmark data and HTML reports are available in:
- \`target/criterion/\` - Individual benchmark reports

EOF

    log_success "Report written to $report_file"
}

# Main function
main() {
    log_info "Starting Foundry benchmark suite..."
    
    # Check dependencies
    check_dependencies
    
    # Install Foundry versions if --force-install is used
    install_foundry_versions
    
    # Run benchmarks and generate report
    run_benchmarks
    
    log_success "Benchmark suite completed successfully!"
    echo ""
    echo "View the results:"
    echo "  - Text report: cat LATEST.md"
}

# Help function
show_help() {
    cat << EOF
Foundry Benchmark Runner

This script runs Criterion-based benchmarks for Foundry commands and generates
a markdown report using criterion-table.

USAGE:
    $0 [OPTIONS]

OPTIONS:
    -h, --help               Show this help message
    -v, --version            Show version information
    --verbose                Show benchmark output (by default output is suppressed)
    --versions <versions>    Comma-separated list of Foundry versions to test
                            (e.g. stable,nightly,v1.2.0)
                            If not specified, uses versions from src/lib.rs
    --force-install          Force installation of Foundry versions
                            By default, assumes versions are already installed

REQUIREMENTS:
    - criterion-table: cargo install criterion-table
    - cargo-criterion: cargo install cargo-criterion
    - Foundry versions must be installed (or use --force-install)

EXAMPLES:
    $0                                          # Run with default versions
    $0 --verbose                                # Show full output
    $0 --versions stable,nightly                # Test specific versions
    $0 --versions stable,nightly --force-install # Install and test versions
    
The script will:
1. Run forge_test, forge_build_no_cache, and forge_build_with_cache benchmarks
2. Generate comparison tables using criterion-table
3. Include system information and Foundry version details
4. Save the complete report to LATEST.md

EOF
}

# Default values
VERBOSE=false
FORCE_INSTALL=false
CUSTOM_VERSIONS=""

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            show_help
            exit 0
            ;;
        -v|--version)
            echo "Foundry Benchmark Runner v1.0.0"
            exit 0
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --force-install)
            FORCE_INSTALL=true
            shift
            ;;
        --versions)
            if [[ -z "$2" ]] || [[ "$2" == --* ]]; then
                log_error "--versions requires a comma-separated list of versions"
                exit 1
            fi
            CUSTOM_VERSIONS="$2"
            shift 2
            ;;
        *)
            log_error "Unknown option: $1"
            echo "Use -h or --help for usage information"
            exit 1
            ;;
    esac
done

main