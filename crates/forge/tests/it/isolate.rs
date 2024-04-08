//! Isolation tests.

use crate::{config::*, test_helpers::TEST_DATA_DEFAULT};
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn test_isolate_record_gas() {
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.isolate = true;
    let runner = TEST_DATA_DEFAULT.runner_with_config(config);
    let filter = Filter::new(".*", ".*", "isolate/RecordGas.t.sol");
    TestConfig::with_filter(runner, filter).run().await;
}
