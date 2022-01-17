.PHONY: format lint test ready

# Repositories for integration tests that will be cloned inside `integration-tests/testdata/REPO` folders
INTEGRATION_TESTS_REPOS = \
	mds1/drai \
	reflexer-labs/geb \
	hexonaut/guni-lev \
	Rari-Capital/solmate \
	Arachnid/solidity-stringutils \
	rari-capital/vaults \
	makerdao/multicall \
	gakonst/lootloose

integration-tests-testdata: $(INTEGRATION_TESTS_REPOS)

$(INTEGRATION_TESTS_REPOS):
	@FOLDER=$(shell dirname "$0")/integration-tests/testdata/$(lastword $(subst /, ,$@));\
	if [ ! -d $$FOLDER ] ; then git clone --depth 1 --recursive https://github.com/$@ $$FOLDER;\
	else cd $$FOLDER; git pull && git submodule update --recursive; fi

testdata: integration-tests-testdata

format: 
	cargo +nightly fmt

lint: 
	cargo +nightly clippy --all-features -- -D warnings

test:
	cargo check
	cargo test
	cargo doc --open

ready: format lint test
