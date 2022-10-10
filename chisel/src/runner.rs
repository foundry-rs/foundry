use bytes::Bytes;
use ethers::prelude::{types::U256, Address};
use revm::{
    return_ok, BlockEnv, CfgEnv, CreateScheme, Database, EVMData, Env, InMemoryDB, Inspector,
    Interpreter, Return, SpecId, TransactOut, TransactTo, TxEnv, EVM,
};

/// The Chisel Runner
#[derive(Debug)]
pub struct ChiselRunner {
    /// The runner database
    pub database: InMemoryDB,
    /// The revm environment config
    pub revm_env: Env,
}

impl Default for ChiselRunner {
    fn default() -> Self {
        Self { database: InMemoryDB::default(), revm_env: Env::default() }
    }
}

impl ChiselRunner {
    /// Returns a mutable reference to the runner's database
    pub fn db_mut(&mut self) -> &mut InMemoryDB {
        &mut self.database
    }

    /// Deploy the REPL contract
    ///
    /// ### Returns
    ///
    /// The address of the deployed repl contract or an error
    pub fn deploy_repl(&mut self, code: Bytes) -> Result<Address, &str> {
        let mut evm = EVM::new();
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

    /// Call a contract's `run()` function and inspect with the [ChiselInspector]
    pub fn run(&mut self, address: Address) {
        let mut evm = EVM::new();
        evm.env = self.build_env(
            Address::zero(),
            TransactTo::Call(address),
            Bytes::from_static(&[0xc0, 0x40, 0x62, 0x26]), // "run()" selector
            U256::zero(),
        );
        evm.database(self.db_mut());

        let mut chisel_logger = ChiselInspector::default();
        evm.inspect(&mut chisel_logger);
        println!("{:?}", chisel_logger.state);

        // TODO
    }

    /// Build an REVM environment.
    /// TODO: Configuration
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

#[derive(Default)]
struct ChiselInspector {
    pub state: Option<(revm::Stack, revm::Memory, Return)>,
}

impl<DB> Inspector<DB> for ChiselInspector
where
    DB: Database,
{
    fn step_end(
        &mut self,
        interp: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _: bool,
        eval: Return,
    ) -> Return {
        // TODO: Only set final state
        // Will need to find the program counter of the final instruction within `run()`
        self.state = Some((interp.stack().clone(), interp.memory.clone(), eval));

        eval
    }
}
