function test() public {
    // https://github.com/foundry-rs/foundry/issues/11905
    timelockController.grantRole(keccak256("EXECUTOR_ROLE"), address(0));
}
