use bytes::Bytes;
use revm::{
    return_ok, BlockEnv, CfgEnv, CreateScheme, Database, Env, InMemoryDB, Return,
    SpecId, TransactOut, TransactTo, TxEnv, EVM,
};
use ethers::prelude::{Address, types::U256};

pub struct ChiselRunner {
    pub database: InMemoryDB,
    pub revm_env: Env,
}

impl Default for ChiselRunner {
    fn default() -> Self {
        Self { database: InMemoryDB::default(), revm_env: Env::default() }
    }
}

impl ChiselRunner {
    pub fn db_mut(&mut self) -> &mut InMemoryDB {
        &mut self.database
    }

    /// Set the balance of an account.
    pub fn set_balance(&mut self, address: Address, amount: U256) -> &mut Self {
        let db = self.db_mut();

        // let mut account = db.basic(address).unwrap().unwrap();
        // account.balance = amount;
        // db.insert_account_info(address, account);
        self
    }

    pub fn deploy_code(&mut self, code: Bytes) -> Result<Address, &str> {
        let mut evm = EVM::new();
        self.set_balance(Address::zero(), U256::MAX);
        evm.env = self.build_env(
            Address::zero(),
            TransactTo::Create(CreateScheme::Create),
            code,
            U256::zero(),
        );
        evm.database(self.db_mut());

        // Send our CREATE transaction
        let result = evm.transact_commit();

        // Check if deployment was successful
        let address = match result.exit_reason {
            return_ok!() => {
                if let TransactOut::Create(_, Some(addr)) = result.out {
                    addr
                } else {
                    return Err("Could not deploy contract!")
                }
            }
            _ => return Err("Could not deploy contract!"),
        };
        Ok(address)
    }

    pub fn call_repl(&self, address: Address) {
        todo!()
    }

    /// Build an REVM environment.
    fn build_env(&self, caller: Address, to: TransactTo, data: Bytes, value: U256) -> Env {
        Env {
            cfg: CfgEnv { chain_id: 1.into(), spec_id: SpecId::LATEST, ..Default::default() },
            block: BlockEnv { basefee: 0.into(), gas_limit: U256::MAX, ..Default::default() },
            tx: TxEnv {
                chain_id: 1.into(),
                caller,
                transact_to: to,
                data,
                value,
                ..Default::default()
            },
        }
    }
}
