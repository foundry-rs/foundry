# Forge versions |stable | revm-24 nightly | revm-27 nightly
export VERSIONS="v1.2.3,nightly-05918765cb239024e9ca396825abb9f46257419a,nightly-13c4502c80ceae8429056eefc1e6a3b1e4e86b53" \
# Repositories
export REPOS="ithacaxyz/account:v0.3.2,Vectorized/solady:v0.1.22,Uniswap/v4-core:59d3ecf,sparkdotfi/spark-psm:v1.0.0" \

export COVERAGE_REPOS="ithacaxyz/account:v0.3.2,Uniswap/v4-core:59d3ecf,sparkdotfi/spark-psm:v1.0.0" \

# Forge test bench

foundry-bench --versions "$VERSIONS" --repos "$REPOS" --benchmarks forge_test,forge_fuzz_test --output-dir ./benches --output-file TEST_BENCH.md && \

# Forge build bench

foundry-bench --versions "$VERSIONS" --repos "$REPOS" --benchmarks forge_build_no_cache,forge_build_with_cache --output-dir ./benches --output-file BUILD_BENCH.md && \

# Coverage bench

foundry-bench --versions "$VERSIONS" --repos "$COVERAGE_REPOS" --benchmarks forge_coverage --output-dir ./benches --output-file COVERAGE_BENCH.md

