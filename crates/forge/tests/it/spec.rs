//! Integration tests for EVM specifications.

use crate::{config::*, test_helpers::TEST_DATA_PARIS};
use foundry_test_utils::Filter;
use revm::primitives::hardfork::SpecId;

#[tokio::test(flavor = "multi_thread")]
async fn test_shanghai_compat() {
    let filter = Filter::new("", "ShanghaiCompat", ".*spec");
    TestConfig::with_filter(TEST_DATA_PARIS.runner(), filter).spec_id(SpecId::SHANGHAI).run().await;
}
