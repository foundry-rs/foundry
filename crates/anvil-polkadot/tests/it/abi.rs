use alloy_sol_types::sol;

sol!(
    #[derive(Debug)]
    SimpleStorage,
    "test-data/SimpleStorage.json"
);

sol!(
    #[derive(Debug)]
    DoubleStorage,
    "test-data/DoubleStorage.json"
);
