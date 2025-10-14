//! Forge tests for migration between EVM and Revive.

use crate::{config::*, test_helpers::TEST_DATA_REVIVE};
use foundry_test_utils::Filter;
use revm::primitives::hardfork::SpecId;

#[tokio::test(flavor = "multi_thread")]
async fn test_revive_balance_migration() {
    let runner = TEST_DATA_REVIVE.runner_revive();
    let filter = Filter::new("testBalanceMigration", "EvmReviveMigrationTest", ".*/revive/.*");

    TestConfig::with_filter(runner, filter).spec_id(SpecId::SHANGHAI).run().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_revive_nonce_migration() {
    let runner = TEST_DATA_REVIVE.runner_revive();
    let filter = Filter::new("testNonceMigration", "EvmReviveMigrationTest", ".*/revive/.*");

    TestConfig::with_filter(runner, filter).spec_id(SpecId::SHANGHAI).run().await;
}

// Enable it after new pallet-revive is being used
// #[tokio::test(flavor = "multi_thread")]
// async fn test_revive_precision_preservation() {
//     let runner = TEST_DATA_REVIVE.runner_revive();
//     let filter = Filter::new("testPrecisionPreservation", "EvmReviveMigrationTest",
// ".*/revive/.*");
//
//     TestConfig::with_filter(runner, filter).spec_id(SpecId::SHANGHAI).run().await;
// }

#[tokio::test(flavor = "multi_thread")]
async fn test_revive_bytecode_migration() {
    let runner = TEST_DATA_REVIVE.runner_revive();
    let filter = Filter::new("testBytecodeMigration", "EvmReviveMigrationTest", ".*/revive/.*");

    TestConfig::with_filter(runner, filter).spec_id(SpecId::SHANGHAI).run().await;
}

// #[tokio::test(flavor = "multi_thread")]
// async fn test_revive_timestamp_migration() {
//     let runner = TEST_DATA_REVIVE.runner_revive();
//     let filter = Filter::new("testTimestampMigration", "EvmReviveMigrationTest", ".*/revive/.*");

//     TestConfig::with_filter(runner, filter).spec_id(SpecId::SHANGHAI).run().await;
// }
