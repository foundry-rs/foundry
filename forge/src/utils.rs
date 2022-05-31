use ethers::{abi::Address, types::U256};
use foundry_evm::executor::{inspector::DEFAULT_CREATE2_DEPLOYER, DatabaseRef, Executor};
use std::str::FromStr;

/// Creates the default CREATE2 Contract Deployer for local tests and scripts.
pub fn deploy_create2_deployer<DB: DatabaseRef>(executor: &mut Executor<DB>) -> eyre::Result<()> {
    let creator = Address::from_str("0x3fAB184622Dc19b6109349B94811493BF2a45362").unwrap();

    let create2_deployer_account = executor.db.basic(DEFAULT_CREATE2_DEPLOYER);

    if create2_deployer_account.code.is_none() ||
        create2_deployer_account.code.as_ref().unwrap().is_empty()
    {
        executor.set_balance(creator, U256::MAX);
        executor.deploy(
            creator,
            hex::decode("604580600e600039806000f350fe7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3").expect("Could not decode create2 deployer init_code").into(),
            U256::zero(),
            None
        )?;
    }
    Ok(())
}
