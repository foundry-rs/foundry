//! Integration tests for EVM specifications.

use crate::{config::*, test_helpers::TEST_DATA_DEFAULT};
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn test_basic_vyper_test() {
    let filter = Filter::new("", "CounterTest", ".*vyper");
    TestConfig::with_filter(TEST_DATA_DEFAULT.runner(), filter).run().await;
}
