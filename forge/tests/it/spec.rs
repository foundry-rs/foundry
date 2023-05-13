use crate::{config::*, test_helpers::filter::Filter};
use forge::revm::primitives::SpecId;

#[test]
fn test_shanghai_compat() {
    let filter = Filter::new("", "ShanghaiCompat", ".*spec");
    TestConfig::filter(filter).evm_spec(SpecId::SHANGHAI).run();
}
