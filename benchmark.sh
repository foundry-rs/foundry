export VERSIONS="v1.3.6,v1.4.0-rc1" \

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

export TEST_ISOLATE="forge_isolate_test" \


echo "===========FORGE TEST AND BUILD BENCHMARKS===========" && \

foundry-bench --versions $VERSIONS --repos $ITHACA_ACCOUNT,$SOLADY_REPO,$UNISWAP_V4_CORE,$SPARK_PSM --benchmarks $TEST,$FUZZ_TEST,$BUILD,$BUILD_CACHE --output-dir ./benches/results --output-file TEST_BUILD.md && \

echo "===========FORGE COVERAGE BENCHMARKS===========" && \

foundry-bench --versions $VERSIONS --repos $ITHACA_ACCOUNT,$UNISWAP_V4_CORE,$SPARK_PSM --benchmarks $COVERAGE --output-dir ./benches/results --output-file COVERAGE.md && \

echo "===========FORGE ISOLATE TEST BENCHMARKS===========" && \

foundry-bench --versions $VERSIONS --repos $SOLADY_REPO,$UNISWAP_V4_CORE,$SPARK_PSM --benchmarks $TEST_ISOLATE --output-dir ./benches/results --output-file ISOLATE_TEST.md && \

echo "===========BENCHMARKS COMPLETED==========="
