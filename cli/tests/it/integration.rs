use foundry_cli_test_utils::forgetest_external;

forgetest_external!(solmate, "Rari-Capital/solmate");
forgetest_external!(geb, "reflexer-labs/geb");
forgetest_external!(stringutils, "Arachnid/solidity-stringutils");
// forgetest_external!(vaults, "Rari-Capital/vaults");
forgetest_external!(multicall, "makerdao/multicall", &["--block-number", "1"]);
forgetest_external!(lootloose, "gakonst/lootloose");
forgetest_external!(lil_web3, "m1guelpf/lil-web3");
forgetest_external!(maple_loan, "maple-labs/loan");

// Forking tests
forgetest_external!(drai, "mds1/drai", 13633752, &["--chain-id", "99"]);
forgetest_external!(gunilev, "hexonaut/guni-lev", 13633752);
forgetest_external!(convex, "mds1/convex-shutdown-simulation", 14445961);
