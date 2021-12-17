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
	@integration-tests/add_test.sh $@

testdata: integration-tests-testdata
