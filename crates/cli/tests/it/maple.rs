use foundry_cli_test_utils::forgetest_external;

// Runs an integration test for maple.
// This is its own test because its extremely heavy, and as a result is not included
// in the normal run.
forgetest_external!(maple, "maple-labs/maple-core-v2");
