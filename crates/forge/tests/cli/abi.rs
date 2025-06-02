#![allow(clippy::too_many_arguments)]
use alloy_sol_types::sol;

sol!(
    #[sol(rpc)]
    ENSRegistry,
    "test-data/ENSRegistry.json"
);

sol!(
    #[sol(rpc)]
    BaseRegistrarImplementation,
    "test-data/BaseRegistrarImplementation.json"
);

sol!(
    #[sol(rpc)]
    DummyOracle,
    "test-data/DummyOracle.json"
);

sol!(
    #[sol(rpc)]
    ETHRegistrarController,
    "test-data/ETHRegistrarController.json"
);

sol!(
    #[sol(rpc)]
    NameWrapper,
    "test-data/NameWrapper.json"
);

sol!(
    #[sol(rpc)]
    PublicResolver,
    "test-data/PublicResolver.json"
);

sol!(
    #[sol(rpc)]
    ReverseRegistrar,
    "test-data/ReverseRegistrar.json"
);

pub mod price_oracle {
    use alloy_sol_types::sol;
    sol!(
        #[sol(rpc)]
        StablePriceOracle,
        "test-data/StablePriceOracle.json"
    );
}
