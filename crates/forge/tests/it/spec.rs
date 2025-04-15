//! Integration tests for EVM specifications.

use crate::{config::*, test_helpers::TEST_DATA_PARIS};
use foundry_evm::revm::primitives::SpecId;
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn test_shanghai_compat() {
    let filter = Filter::new("", "ShanghaiCompat", ".*spec");
    TestConfig::with_filter(TEST_DATA_PARIS.runner(), filter).spec_id(SpecId::SHANGHAI).run().await;
}
