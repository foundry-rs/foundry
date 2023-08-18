//! Heavy integration tests that can take an hour to run or more.
//! All tests are prefixed with heavy so they can be filtered by nextest.

use foundry_test_utils::forgetest_external;

forgetest_external!(heavy_maple, "maple-labs/maple-core-v2");
