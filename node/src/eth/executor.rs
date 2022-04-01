use ethers::types::{transaction::eip2930::AccessList, Address, Bytes, H256, U256};


/// Basic [revm](foundry_evm::revm) Executor abstraction
pub trait Executor {
    type Error;

    fn call(
        source: Address,
        target: Address,
        input: Vec<u8>,
        value: U256,
        gas_limit: u64,
        max_fee_per_gas: Option<U256>,
        max_priority_fee_per_gas: Option<U256>,
        nonce: Option<U256>,
        access_list: AccessList,
    ) -> Result<Bytes, Self::Error>;

    fn create(
        source: Address,
        init: Vec<u8>,
        value: U256,
        gas_limit: u64,
        max_fee_per_gas: Option<U256>,
        max_priority_fee_per_gas: Option<U256>,
        nonce: Option<U256>,
        access_list: AccessList,
    ) -> Result<Address, Self::Error>;

    fn create2(
        source: Address,
        init: Vec<u8>,
        salt: H256,
        value: U256,
        gas_limit: u64,
        max_fee_per_gas: Option<U256>,
        max_priority_fee_per_gas: Option<U256>,
        nonce: Option<U256>,
        access_list: AccessList,
    ) -> Result<Address, Self::Error>;
}

// /// Helper function to execute [Executor] functions
// pub fn execute<F, DB, R>(
//     caller: Address,
//     value: U256,
//     gas_limit: u64,
//     max_fee_per_gas: Option<U256>,
//     max_priority_fee_per_gas: Option<U256>,
//     nonce: Option<U256>,
//     env: revm::Env,
//     db: DB,
//     f: F
// ) where
//     DB: Database + DatabaseCommit,
//     F: FnOnce(EVM<DB>) -> R
// {
//     let mut evm = EVM::new();
//     evm.env = env;
//     evm.database(db);
//     f(evm)
// }
