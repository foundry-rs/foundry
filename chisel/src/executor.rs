use crate::prelude::{ChiselRunner, ChiselSession};
use core::fmt::Debug;
use ethers::{
    types::{Address, U256},
    utils::hex,
};
use ethers_solc::Artifact;
use forge::executor::{Backend, ExecutorBuilder};
use revm::OpCode;
use solang_parser::pt::CodeLocation;

/// Executor implementation for [ChiselSession]
impl ChiselSession {
    /// Runs the REPL contract within the executor
    /// TODO - Proper return type, etc.
    pub fn execute(&mut self) -> Result<(), &str> {
        // Recompile the project and ensure no errors occurred.
        match self.session_source.as_mut().ok_or("Missing Project")?.build() {
            Ok(compiled) => {
                if let Some((_, contract)) =
                    compiled.compiler_output.contracts_into_iter().find(|(name, _)| name.eq("REPL"))
                {
                    // This *should* never panic.
                    let bytecode =
                        contract.get_bytecode_bytes().expect("No bytecode for contract.");

                    let final_statement = compiled.intermediate.statements.last().unwrap();
                    let final_pc = {
                        let source_loc = final_statement.loc();
                        let offset = source_loc.start();
                        let length = source_loc.end() - source_loc.start();
                        contract
                            .get_source_map_deployed()
                            .unwrap()
                            .unwrap()
                            .into_iter()
                            .zip(InstructionIter::new(&bytecode))
                            .filter(|(s, _)| s.offset == offset && s.length == length)
                            .map(|(_, i)| i.pc)
                            .max()
                            .unwrap_or_default()
                    };

                    dbg!(final_pc);

                    // Create a new runner
                    let mut runner = self.prepare_runner(final_pc);

                    // Run w/ no libraries for now
                    let res = runner.run(&[], bytecode.into_owned());
                    println!("{:?}", &res);
                    if let Some(state) = res.unwrap().1.state {
                        dbg!(state.0);
                    }
                    Ok(())
                } else {
                    Err("Could not find artifact for REPL contract.")
                }
            }
            Err(err) => {
                // TODO: Remove this debug stuff
                let err = err.to_string();
                dbg!(err);
                Err("test")
            }
        }
    }

    /// Prepare a runner for the Chisel REPL environment
    pub fn prepare_runner(&self, final_pc: usize) -> ChiselRunner {
        // Spawn backend with no fork at the moment
        // TODO: Make the backend persistent, shouldn't spawn a new one each time.
        let backend = Backend::spawn(None);

        // Build a new executor
        // TODO: Configurability, custom inspector for `step_end`
        let executor = ExecutorBuilder::default()
            .with_chisel_state(final_pc)
            .set_tracing(true)
            .with_spec(revm::SpecId::LATEST)
            .with_gas_limit(u64::MAX.into())
            .build(backend);

        ChiselRunner::new(executor, U256::MAX, Address::zero())
    }
}

// [Instruction] & [InstructionIter] ripped from
// [soli](https://github.com/jpopesculian/soli)
// =============================================

#[derive(Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
struct Instruction {
    pub pc: usize,
    pub opcode: u8,
    pub data: [u8; 32],
    pub data_len: u8,
}

impl Instruction {
    fn data(&self) -> &[u8] {
        &self.data[..self.data_len as usize]
    }
}

impl Debug for Instruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instruction")
            .field("pc", &self.pc)
            .field(
                "opcode",
                &format_args!(
                    "{}",
                    OpCode::try_from_u8(self.opcode)
                        .map(|op| op.as_str().to_owned())
                        .unwrap_or_else(|| format!("0x{}", hex::encode(&[self.opcode])))
                ),
            )
            .field("data", &format_args!("0x{}", hex::encode(self.data())))
            .finish()
    }
}

struct InstructionIter<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> InstructionIter<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl<'a> Iterator for InstructionIter<'a> {
    type Item = Instruction;
    fn next(&mut self) -> Option<Self::Item> {
        let pc = self.offset;
        self.offset += 1;
        let opcode = *self.bytes.get(pc)?;
        let (data, data_len) = if matches!(opcode, 0x60..=0x7F) {
            let mut data = [0; 32];
            let data_len = (opcode - 0x60 + 1) as usize;
            data[..data_len].copy_from_slice(&self.bytes[self.offset..self.offset + data_len]);
            self.offset += data_len;
            (data, data_len as u8)
        } else {
            ([0; 32], 0)
        };
        Some(Instruction { pc, opcode, data, data_len })
    }
}
