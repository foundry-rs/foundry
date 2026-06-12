#!/bin/bash

versions="v1.5.1,v1.7.0"

# Repositories
ITHACA_ACCOUNT="ithacaxyz/account:v0.5.7"
SOLADY_REPO="vectorized/solady:v0.1.26 --nmc 'LifebuoyTest|LibBitTest|Base58Test'"
AAVE_V4="aave/aave-v4:af1f0f2ba323ac6fbaaee3abf6be060c78e22d35"
UNISWAP_V4_CORE="uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75 --nmc TickMathTestTest"
SPARK_PSM="sparkdotfi/spark-psm:v1.0.0 --nmc PSMInvariants_TimeBasedRateSetting_WithTransfers_WithPocketSetting"

SOLADY_ISOLATE="vectorized/solady:v0.1.26 --nmc 'SafeTransferLibTest|LifebuoyTest|LibBitTest|Base58Test|LibStringTest'"
ITHACA_ISOLATE="ithacaxyz/account:v0.5.7 --nmc SimulateExecuteTest"

SOLADY_BUILD="vectorized/solady:v0.1.26"
UNISWAP_BUILD="uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75"
SPARK_PSM_BUILD="sparkdotfi/spark-psm:v1.0.0"

# Benches
echo "===========FORGE TEST BENCHMARKS==========="

foundry-bench --versions "$versions" \
    --repos "$ITHACA_ACCOUNT,$SOLADY_REPO,$AAVE_V4,$UNISWAP_V4_CORE,$SPARK_PSM" \
    --benchmarks forge_test,forge_fuzz_test \
    --output-dir ./benches \
    --output-file forge_test_bench.md

echo "===========FORGE ISOLATE TEST BENCHMARKS==========="

foundry-bench --versions "$versions" \
    --repos "$ITHACA_ISOLATE,$SOLADY_ISOLATE,$AAVE_V4,$UNISWAP_V4_CORE,$SPARK_PSM" \
    --benchmarks forge_isolate_test \
    --output-dir ./benches \
    --output-file forge_isolate_test_bench.md

echo "===========FORGE BUILD BENCHMARKS==========="

foundry-bench --versions "$versions" \
    --repos "$ITHACA_ACCOUNT,$SOLADY_BUILD,$AAVE_V4,$UNISWAP_BUILD,$SPARK_PSM_BUILD" \
    --benchmarks forge_build_no_cache,forge_build_with_cache \
    --output-dir ./benches \
    --output-file forge_build_bench.md

echo "===========FORGE COVERAGE BENCHMARKS==========="

foundry-bench --versions "$versions" \
    --repos "$ITHACA_ACCOUNT,$AAVE_V4,$UNISWAP_BUILD,$SPARK_PSM_BUILD" \
    --benchmarks forge_coverage \
    --output-dir ./benches \
    --output-file forge_coverage_bench.md

echo "===========BENCHMARKS COMPLETED==========="
