//! Integration tests for EVM specifications.

use crate::config::*;
use foundry_evm::revm::primitives::SpecId;
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn test_shanghai_compat() {
    let filter = Filter::new("", "ShanghaiCompat", ".*spec");
    TestConfig::filter(filter).await.evm_spec(SpecId::SHANGHAI).run().await;
}
