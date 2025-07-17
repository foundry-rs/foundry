# Forge versions |stable | revm-24 nightly | revm-27 nightly
export VERSIONS="v1.2.3,nightly-05918765cb239024e9ca396825abb9f46257419a,nightly-0af43412f809c998d8b2fe69a1c9a789b7ebd085" \

# Repositories

export ITHACA_ACCOUNT="ithacaxyz/account:v0.3.2" \

export SOLADY_REPO="Vectorized/solady:v0.1.22" \

export UNISWAP_V4_CORE="Uniswap/v4-core:59d3ecf" \

export SPARK_PSM="sparkdotfi/spark-psm:v1.0.0" \

# Benches 

export TEST="forge_test" \

export FUZZ_TEST="forge_fuzz_test" \

export BUILD="forge_build_no_cache" \

export BUILD_CACHE="forge_build_with_cache" \

export COVERAGE="forge_coverage" \

# Results Dir

export TEST_RESUTLS_DIR="./benches/results/test" \

export BUILD_RESULTS_DIR="./benches/results/build" \

export COVERAGE_RESULTS_DIR="./benches/results/coverage" \

# Bench every repo in isolation for each command to get the true performance.

# Forge Tests

echo "================== Forge Tests ==================" && \

foundry-bench --versions "$VERSIONS" --repos "$ITHACA_ACCOUNT" --benchmarks $TEST --output-dir $TEST_RESUTLS_DIR --output-file ITHACA_ACCOUNT.md && \
foundry-bench --versions "$VERSIONS" --repos "$SOLADY_REPO" --benchmarks $TEST --output-dir $TEST_RESUTLS_DIR --output-file SOLADY.md && \
foundry-bench --versions "$VERSIONS" --repos "$UNISWAP_V4_CORE" --benchmarks $TEST --output-dir $TEST_RESUTLS_DIR --output-file UNISWAP_V4_CORE.md && \
foundry-bench --versions "$VERSIONS" --repos "$SPARK_PSM" --benchmarks $TEST --output-dir $TEST_RESUTLS_DIR --output-file SPARK_PSM.md && \

# Forge Fuzz Tests

echo "================== Forge Fuzz Tests ==================" && \

foundry-bench --versions "$VERSIONS" --repos "$ITHACA_ACCOUNT" --benchmarks $FUZZ_TEST --output-dir $TEST_RESUTLS_DIR --output-file ITHACA_ACCOUNT_fuzz.md && \
foundry-bench --versions "$VERSIONS" --repos "$SOLADY_REPO" --benchmarks $FUZZ_TEST --output-dir $TEST_RESUTLS_DIR --output-file SOLADY_fuzz.md && \
foundry-bench --versions "$VERSIONS" --repos "$UNISWAP_V4_CORE" --benchmarks $FUZZ_TEST --output-dir $TEST_RESUTLS_DIR --output-file UNISWAP_V4_CORE_fuzz.md && \
foundry-bench --versions "$VERSIONS" --repos "$SPARK_PSM" --benchmarks $FUZZ_TEST --output-dir $TEST_RESUTLS_DIR --output-file SPARK_PSM_fuzz.md && \

# Forge Build

echo "================== Forge Build ==================" && \

foundry-bench --versions "$VERSIONS" --repos "$ITHACA_ACCOUNT" --benchmarks $BUILD --output-dir $BUILD_RESULTS_DIR --output-file ITHACA_ACCOUNT.md && \
foundry-bench --versions "$VERSIONS" --repos "$SOLADY_REPO" --benchmarks $BUILD --output-dir $BUILD_RESULTS_DIR --output-file SOLADY.md && \
foundry-bench --versions "$VERSIONS" --repos "$UNISWAP_V4_CORE" --benchmarks $BUILD --output-dir $BUILD_RESULTS_DIR --output-file UNISWAP_V4_CORE.md && \
foundry-bench --versions "$VERSIONS" --repos "$SPARK_PSM" --benchmarks $BUILD --output-dir $BUILD_RESULTS_DIR --output-file SPARK_PSM.md && \

# Forge Build with Cache

echo "================== Forge Build with Cache ==================" && \

foundry-bench --versions "$VERSIONS" --repos "$ITHACA_ACCOUNT" --benchmarks $BUILD_CACHE --output-dir $BUILD_RESULTS_DIR --output-file ITHACA_ACCOUNT_cache.md && \
foundry-bench --versions "$VERSIONS" --repos "$SOLADY_REPO" --benchmarks $BUILD_CACHE --output-dir $BUILD_RESULTS_DIR --output-file SOLADY_cache.md && \
foundry-bench --versions "$VERSIONS" --repos "$UNISWAP_V4_CORE" --benchmarks $BUILD_CACHE --output-dir $BUILD_RESULTS_DIR --output-file UNISWAP_V4_CORE_cache.md && \
foundry-bench --versions "$VERSIONS" --repos "$SPARK_PSM" --benchmarks $BUILD_CACHE --output-dir $BUILD_RESULTS_DIR --output-file SPARK_PSM_cache.md && \

# Coverage

echo "================== Forge Coverage ==================" && \

foundry-bench --versions "$VERSIONS" --repos "$ITHACA_ACCOUNT" --benchmarks $COVERAGE --output-dir $COVERAGE_RESULTS_DIR --output-file ITHACA_ACCOUNT.md && \
foundry-bench --versions "$VERSIONS" --repos "$UNISWAP_V4_CORE" --benchmarks $COVERAGE --output-dir $COVERAGE_RESULTS_DIR --output-file UNISWAP_V4_CORE.md && \
foundry-bench --versions "$VERSIONS" --repos "$SPARK_PSM" --benchmarks $COVERAGE --output-dir $COVERAGE_RESULTS_DIR --output-file SPARK_PSM.md && \

echo "================== Forge Benchmarks Completed =================="
