#!/bin/bash

# Foundry Multi-Version Benchmarking Suite using hyperfine
# This script benchmarks forge test and forge build commands across multiple repositories
# and multiple Foundry versions for comprehensive performance comparison

set -e

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCHMARK_DIR="${SCRIPT_DIR}/benchmark_repos"
RESULTS_DIR="${SCRIPT_DIR}/benchmark_results"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
RESULTS_FILE="${RESULTS_DIR}/foundry_multi_version_benchmark_${TIMESTAMP}.md"
JSON_RESULTS_DIR="${RESULTS_DIR}/json_${TIMESTAMP}"

# Foundry versions to benchmark (can be modified via command line)
DEFAULT_FOUNDRY_VERSIONS=("stable" "nightly-ac0411d0e3b9632247c9aea9535472eda09a57ae" "nightly") # nightly-ac0 with linter not included in forge build.
FOUNDRY_VERSIONS=("${FOUNDRY_VERSIONS[@]:-${DEFAULT_FOUNDRY_VERSIONS[@]}}")

# Repository configurations
REPO_NAMES=(
    "account"
    "v4-core"
    "solady"
    "morpho-blue"
    "spark-psm"
)

REPO_URLS=(
    "https://github.com/ithacaxyz/account"
    "https://github.com/Uniswap/v4-core"
    "https://github.com/Vectorized/solady"
    "https://github.com/morpho-org/morpho-blue"
    "https://github.com/sparkdotfi/spark-psm"
)

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
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

# Install foundryup if not present
install_foundryup() {
    if ! command -v foundryup &> /dev/null; then
        log_info "Installing foundryup..."
        curl -L https://foundry.paradigm.xyz | bash
        # Source the bashrc/profile to get foundryup in PATH
        export PATH="$HOME/.foundry/bin:$PATH"
    fi
}

# Install a specific Foundry version
install_foundry_version() {
    local version=$1
    log_info "Installing Foundry version: $version"
    
    # Let foundryup handle any version format and determine validity
    if foundryup --install "$version"; then
        # Verify installation
        local installed_version=$(forge --version | head -n1 || echo "unknown")
        log_success "Installed Foundry: $installed_version"
        return 0
    else
        log_error "Failed to install Foundry $version"
        return 1
    fi
}

# Check if required tools are installed
check_dependencies() {
    local missing_deps=()
    
    if ! command -v hyperfine &> /dev/null; then
        missing_deps+=("hyperfine")
    fi
    
    if ! command -v git &> /dev/null; then
        missing_deps+=("git")
    fi
    
    if ! command -v curl &> /dev/null; then
        missing_deps+=("curl")
    fi
    
    if [ ${#missing_deps[@]} -ne 0 ]; then
        log_error "Missing required dependencies: ${missing_deps[*]}"
        log_info "Install hyperfine: https://github.com/sharkdp/hyperfine#installation"
        exit 1
    fi
    
    # Install foundryup if needed
    install_foundryup
}

# Setup directories
setup_directories() {
    log_info "Setting up benchmark directories..."
    mkdir -p "$BENCHMARK_DIR"
    mkdir -p "$RESULTS_DIR"
    mkdir -p "$JSON_RESULTS_DIR"
}

# Clone or update repository
clone_or_update_repo() {
    local name=$1
    local url=$2
    local repo_dir="${BENCHMARK_DIR}/${name}"
    
    if [ -d "$repo_dir" ]; then
        log_info "Updating existing repository: $name"
        cd "$repo_dir"
        git pull origin main 2>/dev/null || git pull origin master 2>/dev/null || true
        cd - > /dev/null
    else
        log_info "Cloning repository: $name"
        git clone "$url" "$repo_dir"
    fi
}

# Install dependencies for a repository
install_dependencies() {
    local repo_dir=$1
    local repo_name=$2
    
    log_info "Installing dependencies for $repo_name..."
    cd "$repo_dir"
    
    # Install forge dependencies
    if [ -f "foundry.toml" ] || [ -f "forge.toml" ]; then
        forge install 2>/dev/null || true
    fi
    
    # Install npm dependencies if package.json exists
    if [ -f "package.json" ]; then
        if command -v npm &> /dev/null; then
            npm install 2>/dev/null || true
        fi
    fi
    
    cd - > /dev/null
}

# Run benchmarks for a single repository with a specific Foundry version
benchmark_repository_for_version() {
    local repo_name=$1
    local version=$2
    local repo_dir="${BENCHMARK_DIR}/${repo_name}"
    local log_prefix="[$repo_name:$version]"
    
    # Create a unique log file for this repo+version combination
    local log_file="${JSON_RESULTS_DIR}/${repo_name}_${version//[^a-zA-Z0-9]/_}_benchmark.log"
    
    {
        echo "$(date): Starting benchmark for $repo_name with Foundry $version"
        
        if [ ! -d "$repo_dir" ]; then
            echo "ERROR: Repository directory not found: $repo_dir"
            return 1
        fi
        
        cd "$repo_dir"
        
        # Check if it's a valid Foundry project
        if [ ! -f "foundry.toml" ]; then
            echo "WARN: No foundry.toml found in $repo_name, skipping..."
            cd - > /dev/null
            return 0
        fi
        
        # Clean version string for filenames (remove 'v' prefix, replace '.' with '_')
        local clean_version="${version//v/}"
        clean_version="${clean_version//\./_}"
        
        local version_results_dir="${JSON_RESULTS_DIR}/${repo_name}_${clean_version}"
        mkdir -p "$version_results_dir"
        
        echo "Running benchmarks for $repo_name with Foundry $version..."
        
        # Benchmark 1: forge test
        echo "Running 'forge test' benchmark..."
        if hyperfine \
            --runs 5 \
            --prepare 'forge build' \
            --warmup 1 \
            --export-json "${version_results_dir}/test_results.json" \
            "forge test" 2>>"$log_file.error"; then
            echo "✓ forge test completed"
        else
            echo "✗ forge test failed"
        fi
        
        # Benchmark 2: forge build (no cache)
        echo "Running 'forge build' (no cache) benchmark..."
        if hyperfine \
            --runs 5 \
            --prepare 'forge clean' \
            --export-json "${version_results_dir}/build_no_cache_results.json" \
            "forge build" 2>>"$log_file.error"; then
            echo "✓ forge build (no cache) completed"
        else
            echo "✗ forge build (no cache) failed"
        fi
        
        # Benchmark 3: forge build (with cache)
        echo "Running 'forge build' (with cache) benchmark..."
        if hyperfine \
            --runs 5 \
            --prepare 'forge build' \
            --warmup 1 \
            --export-json "${version_results_dir}/build_with_cache_results.json" \
            "forge build" 2>>"$log_file.error"; then
            echo "✓ forge build (with cache) completed"
        else
            echo "✗ forge build (with cache) failed"
        fi
        
        # Store version info for this benchmark
        forge --version | head -n1 > "${version_results_dir}/forge_version.txt" 2>/dev/null || echo "unknown" > "${version_results_dir}/forge_version.txt"
        
        cd - > /dev/null
        echo "$(date): Completed benchmark for $repo_name with Foundry $version"
        
    } > "$log_file" 2>&1
}

# Run benchmarks for all repositories in parallel for each Foundry version
benchmark_all_repositories_parallel() {
    for version in "${FOUNDRY_VERSIONS[@]}"; do
        log_info "Installing Foundry version: $version"
        
        # Install the specific version once for all repositories
        install_foundry_version "$version" || {
            log_warn "Failed to install Foundry $version, skipping all repositories for this version..."
            continue
        }
        
        log_info "Starting parallel benchmarks for all repositories with Foundry $version"
        
        # Launch all repositories in parallel
        local pids=()
        local failed_repos=()
        
        for repo_name in "${REPO_NAMES[@]}"; do
            # Check if repo directory exists and is valid before starting background process
            local repo_dir="${BENCHMARK_DIR}/${repo_name}"
            if [ ! -d "$repo_dir" ]; then
                log_warn "Repository directory not found: $repo_dir, skipping..."
                continue
            fi
            
            if [ ! -f "${repo_dir}/foundry.toml" ]; then
                log_warn "No foundry.toml found in $repo_name, skipping..."
                continue
            fi
            
            log_info "Launching background benchmark for $repo_name..."
            benchmark_repository_for_version "$repo_name" "$version" &
            local pid=$!
            pids+=($pid)
            echo "$repo_name:$pid" >> "${JSON_RESULTS_DIR}/parallel_pids_${version//[^a-zA-Z0-9]/_}.txt"
        done
        
        # Wait for all repositories to complete
        log_info "Waiting for ${#pids[@]} parallel benchmarks to complete for Foundry $version..."
        local completed=0
        local total=${#pids[@]}
        
        for pid in "${pids[@]}"; do
            if wait "$pid"; then
                completed=$((completed + 1))
                log_info "Progress: $completed/$total repositories completed for Foundry $version"
            else
                log_warn "One benchmark process failed (PID: $pid)"
            fi
        done
        
        log_success "All repositories completed for Foundry $version ($completed/$total successful)"
        
        # Show summary of log files created
        log_info "Individual benchmark logs available in: ${JSON_RESULTS_DIR}/*_${version//[^a-zA-Z0-9]/_}_benchmark.log"
    done
}


# Extract mean time from JSON result file
extract_mean_time() {
    local json_file=$1
    if [ -f "$json_file" ]; then
        # Extract mean time in seconds, format to 3 decimal places
        python3 -c "
import json, sys
try:
    with open('$json_file') as f:
        data = json.load(f)
        mean_time = data['results'][0]['mean']
        print(f'{mean_time:.3f}')
except:
    print('N/A')
" 2>/dev/null || echo "N/A"
    else
        echo "N/A"
    fi
}

# Get Foundry version string from file
get_forge_version() {
    local version_file=$1
    if [ -f "$version_file" ]; then
        cat "$version_file" | sed 's/forge //' | sed 's/ (.*//'
    else
        echo "unknown"
    fi
}

# Compile results into markdown with comparison tables
compile_results() {
    log_info "Compiling multi-version benchmark results..."
    
    cat > "$RESULTS_FILE" << EOF
# Forge Benchmarking Results

**Generated on:** $(date)
**Hyperfine Version:** $(hyperfine --version)
**Foundry Versions Tested:** ${FOUNDRY_VERSIONS[*]}
**Repositories Tested:** ${REPO_NAMES[*]}

## Summary

This report contains comprehensive benchmarking results comparing different Foundry versions across multiple projects.
The following benchmarks were performed:

1. **forge test** - Running the test suite (5 runs, 1 warmup)
2. **forge build (no cache)** - Clean build without cache (5 runs, cache cleaned after each run)
3. **forge build (with cache)** - Build with warm cache (5 runs, 1 warmup)

---

## Performance Comparison Tables

EOF

    # Create unified comparison tables for each benchmark type
    local benchmark_types=("test" "build_no_cache" "build_with_cache")
    local benchmark_names=("forge test" "forge build (no cache)" "forge build (with cache)")
    
    for i in "${!benchmark_types[@]}"; do
        local bench_type="${benchmark_types[$i]}"
        local bench_name="${benchmark_names[$i]}"
        
        echo "### $bench_name" >> "$RESULTS_FILE"
        echo "" >> "$RESULTS_FILE"
        echo "Mean execution time in seconds (lower is better):" >> "$RESULTS_FILE"
        echo "" >> "$RESULTS_FILE"
        
        # Create table header with proper column names
        local header_row="| Project"
        for version in "${FOUNDRY_VERSIONS[@]}"; do
            header_row+=" | $version (s)"
        done
        header_row+=" |"
        echo "$header_row" >> "$RESULTS_FILE"
        
        # Create table separator with proper alignment
        local separator_row="|------"
        for version in "${FOUNDRY_VERSIONS[@]}"; do
            separator_row+="|--------:"
        done
        separator_row+="|"
        echo "$separator_row" >> "$RESULTS_FILE"
        
        # Add data rows
        for repo_name in "${REPO_NAMES[@]}"; do
            local data_row="| **$repo_name**"
            
            for version in "${FOUNDRY_VERSIONS[@]}"; do
                local clean_version="${version//v/}"
                clean_version="${clean_version//\./_}"
                local version_results_dir="${JSON_RESULTS_DIR}/${repo_name}_${clean_version}"
                local json_file="${version_results_dir}/${bench_type}_results.json"
                
                local mean_time=$(extract_mean_time "$json_file")
                data_row+=" | $mean_time"
            done
            data_row+=" |"
            echo "$data_row" >> "$RESULTS_FILE"
        done
        echo "" >> "$RESULTS_FILE"
        echo "" >> "$RESULTS_FILE"
    done
    
    # Add detailed version information
    echo "## Foundry Version Details" >> "$RESULTS_FILE"
    echo "" >> "$RESULTS_FILE"
    
    for version in "${FOUNDRY_VERSIONS[@]}"; do
        echo "### $version" >> "$RESULTS_FILE"
        echo "" >> "$RESULTS_FILE"
        
        # Find any version file to get the detailed version info
        local clean_version="${version//v/}"
        clean_version="${clean_version//\./_}"
        
        for repo_name in "${REPO_NAMES[@]}"; do
            local version_file="${JSON_RESULTS_DIR}/${repo_name}_${clean_version}/forge_version.txt"
            if [ -f "$version_file" ]; then
                echo "\`\`\`" >> "$RESULTS_FILE"
                cat "$version_file" >> "$RESULTS_FILE"
                echo "\`\`\`" >> "$RESULTS_FILE"
                break
            fi
        done
        echo "" >> "$RESULTS_FILE"
    done
    
    # Add notes and system info
    cat >> "$RESULTS_FILE" << EOF

## Notes

- All benchmarks were run with hyperfine
- **forge test**: 3 runs with 1 warmup per version
- **forge build (no cache)**: 3 runs with cache cleanup after each run
- **forge build (with cache)**: 5 runs with 1 warmup on pre-warmed cache
- Results show mean execution time in seconds
- N/A indicates benchmark failed or data unavailable

## System Information

- **OS:** $(uname -s)
- **Architecture:** $(uname -m)
- **Date:** $(date)

## Raw Data

Raw JSON benchmark data is available in: \`$JSON_RESULTS_DIR\`

EOF
}

# Cleanup temporary files
cleanup() {
    # Clean up any temporary files (currently none used in multi-version approach)
    log_info "Cleanup completed"
}

# Main execution
main() {
    log_info "Starting Foundry Multi-Version Benchmarking Suite..."
    log_info "Testing Foundry versions: ${FOUNDRY_VERSIONS[*]}"
    log_info "Testing repositories: ${REPO_NAMES[*]}"
    
    # Setup
    check_dependencies
    setup_directories
    
    # Ensure cleanup on exit
    trap cleanup EXIT
    
    # Clone/update repositories
    for i in "${!REPO_NAMES[@]}"; do
        clone_or_update_repo "${REPO_NAMES[$i]}" "${REPO_URLS[$i]}"
        install_dependencies "${BENCHMARK_DIR}/${REPO_NAMES[$i]}" "${REPO_NAMES[$i]}"
    done
    
    # Run benchmarks in parallel
    log_info "Using parallel execution mode"
    benchmark_all_repositories_parallel
    
    # Compile results
    compile_results
    
    log_success "Multi-version benchmarking complete!"
    log_success "Results saved to: $RESULTS_FILE"
    log_success "Raw JSON data saved to: $JSON_RESULTS_DIR"
    log_info "You can view the results with: cat $RESULTS_FILE"
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --versions)
                shift
                if [[ $# -eq 0 ]]; then
                    log_error "--versions requires a space-separated list of versions"
                    exit 1
                fi
                # Read versions until next flag or end of args
                FOUNDRY_VERSIONS=()
                while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                    FOUNDRY_VERSIONS+=("$1")
                    shift
                done
                ;;
            --help|-h)
                echo "Foundry Multi-Version Benchmarking Suite"
                echo ""
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "OPTIONS:"
                echo "  --help, -h                Show this help message"
                echo "  --version, -v             Show version information"
                echo "  --versions <v1> <v2> ...  Specify Foundry versions to benchmark"
                echo "                            (default: stable nightly v1.0.0)"
                echo ""
                echo "EXAMPLES:"
                echo "  $0                                    # Use default versions (parallel)"
                echo "  $0 --versions stable nightly         # Benchmark stable and nightly only"
                echo "  $0 --versions v1.0.0 v1.1.0 v1.2.0  # Benchmark specific versions"
                echo ""
                echo "This script benchmarks forge test and forge build commands across"
                echo "multiple Foundry repositories and versions using hyperfine."
                echo ""
                echo "EXECUTION MODE:"
                echo "  - Parallel: Install each Foundry version once, then run all repositories"
                echo "    in parallel for that version. This provides much better performance."
                echo ""
                echo "Supported version formats:"
                echo "  - stable, nightly (special tags)"
                echo "  - v1.0.0, v1.1.0, etc. (specific versions)"
                echo "  - nightly-<commit-hash> (specific nightly builds)"
                echo "  - Any format supported by foundryup"
                echo ""
                echo "The script will:"
                echo "  1. Install foundryup if not present"
                echo "  2. Clone/update target repositories"
                echo "  3. Install each specified Foundry version"
                echo "  4. Run benchmarks for each repo with each version in parallel"
                echo "  5. Generate comparison tables in markdown format"
                exit 0
                ;;
            --version|-v)
                echo "Foundry Multi-Version Benchmarking Suite v2.0.0"
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                echo "Use --help for usage information"
                exit 1
                ;;
        esac
    done
}

# Handle command line arguments
if [[ $# -gt 0 ]]; then
    parse_args "$@"
fi

main
