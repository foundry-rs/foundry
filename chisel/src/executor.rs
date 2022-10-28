//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use crate::prelude::{ChiselResult, ChiselRunner, SessionSource};
use core::fmt::Debug;
use ethers::{
    abi::{ethabi, ParamType, Token},
    types::{Address, I256, U256},
    utils::hex,
};
use ethers_solc::Artifact;
use eyre::Result;
use forge::executor::{inspector::CheatsConfig, Backend, ExecutorBuilder};
use revm::OpCode;
use solang_parser::pt::{self, CodeLocation};
use yansi::Paint;

/// Executor implementation for [SessionSource]
impl SessionSource {
    /// Runs the source with the [ChiselRunner]
    pub async fn execute(&mut self) -> Result<(Address, ChiselResult)> {
        // Recompile the project and ensure no errors occurred.
        match self.build() {
            Ok(compiled) => {
                if let Some((_, contract)) =
                    compiled.compiler_output.contracts_into_iter().find(|(name, _)| name.eq("REPL"))
                {
                    // These *should* never panic.
                    let bytecode =
                        contract.get_bytecode_bytes().expect("No bytecode for contract.");
                    let deployed_bytecode = contract
                        .get_deployed_bytecode_bytes()
                        .expect("No deployed bytecode for contract.");

                    // Find the last statement within the "run()" method.
                    if let Some(final_statement) = compiled.intermediate.statements.last() {
                        let final_pc = {
                            let source_loc = final_statement.loc();
                            let offset = source_loc.start();
                            let length = source_loc.end() - source_loc.start();
                            contract
                                .get_source_map_deployed()
                                .unwrap()
                                .unwrap()
                                .into_iter()
                                .zip(InstructionIter::new(&deployed_bytecode))
                                .filter(|(s, _)| s.offset == offset && s.length == length)
                                .map(|(_, i)| i.pc)
                                .max()
                                .unwrap_or_default()
                        };

                        // Create a new runner
                        let mut runner = self.prepare_runner(final_pc).await;

                        // Return [ChiselResult] or bubble up error
                        match runner.run(bytecode.into_owned()) {
                            Ok(res) => Ok(res),
                            Err(e) => Err(e),
                        }
                    } else {
                        // Return a default result if no statements are present.
                        Ok((Address::zero(), ChiselResult::default()))
                    }
                } else {
                    eyre::bail!("Failed to find REPL contract!")
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Inspect a contract element inside of the current session
    pub async fn inspect(&mut self, item: &str) -> Result<String> {
        match self.clone_with_new_line(format!("bytes memory inspectoor = abi.encode({item})")) {
            Ok((mut source, _)) => match source.execute().await {
                Ok((_, res)) => {
                    if let Some((stack, memory, _)) = res.state {
                        let (ty, _) = if let Some(def) = source
                            .generated_output
                            .as_ref()
                            .unwrap()
                            .intermediate
                            .variable_definitions
                            .get(item)
                        {
                            def
                        } else {
                            eyre::bail!("`{item}` definition could not be found");
                        };
                        let ty = if let Some(ty) =
                            Type::from_expression(ty).and_then(|ty| ty.as_ethabi())
                        {
                            ty
                        } else {
                            eyre::bail!("Identifer type currently not supported");
                        };
                        let memory_offset = if let Some(offset) = stack.data().last() {
                            offset.as_usize()
                        } else {
                            eyre::bail!("No result found");
                        };
                        if memory_offset + 32 > memory.len() {
                            eyre::bail!("Memory size insufficient");
                        }
                        let data = &memory.data()[memory_offset + 32..];
                        let token = match ethabi::decode(&[ty], data) {
                            Ok(mut tokens) => {
                                if let Some(token) = tokens.pop() {
                                    token
                                } else {
                                    eyre::bail!("No tokens decoded");
                                }
                            }
                            Err(err) => {
                                eyre::bail!("Could not decode ABI: {err}")
                            }
                        };

                        Ok(format_token(token))
                    } else {
                        eyre::bail!("No state present")
                    }
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    /// Prepare a runner for the Chisel REPL environment
    async fn prepare_runner(&mut self, final_pc: usize) -> ChiselRunner {
        let env = self.config.evm_opts.evm_env().await;

        // Create an in-memory backend
        let backend = self.config.backend.take().unwrap_or_else(|| {
            let backend = Backend::spawn(
                self.config.evm_opts.get_fork(&self.config.foundry_config, env.clone()),
            );
            self.config.backend = Some(backend.clone());
            backend
        });

        // Build a new executor
        let executor = ExecutorBuilder::default()
            .with_config(env)
            .with_chisel_state(final_pc)
            .set_tracing(true)
            .with_spec(foundry_cli::utils::evm_spec(&self.config.foundry_config.evm_version))
            .with_gas_limit(self.config.evm_opts.gas_limit())
            .with_cheatcodes(CheatsConfig::new(&self.config.foundry_config, &self.config.evm_opts))
            .build(backend);

        // Create a [ChiselRunner] with a default balance of [U256::MAX] and
        // the sender [Address::zero].
        ChiselRunner::new(executor, U256::MAX, Address::zero())
    }
}

/// Formats a [Token] into an inspection message
/// TODO: Verbosity option
fn format_token(token: Token) -> String {
    match token {
        Token::Address(a) => {
            format!(
                "Type: {}\n└ Data: {}",
                Paint::red("address"),
                Paint::cyan(format!("0x{:x}", a))
            )
        }
        Token::FixedBytes(b) => {
            format!(
                "Type: {}\n└ Data: {}",
                Paint::red("bytes32"),
                Paint::cyan(format!("0x{}", hex::encode(b)))
            )
        }
        Token::Int(i) => {
            format!(
                "Type: {}\n├ Hex: {}\n└ Decimal: {}",
                Paint::red("int"),
                Paint::cyan(format!("0x{:x}", i)),
                Paint::cyan(I256::from_raw(i))
            )
        }
        Token::Uint(i) => {
            format!(
                "Type: {}\n├ Hex: {}\n└ Decimal: {}",
                Paint::red("uint"),
                Paint::cyan(format!("0x{:x}", i)),
                Paint::cyan(i)
            )
        }
        Token::Bool(b) => {
            format!("Type: {}\n└ Value: {}", Paint::red("bool"), Paint::cyan(b))
        }
        Token::String(_) | Token::Bytes(_) => {
            let hex = hex::encode(ethers::abi::encode(&[token.clone()]));
            let s = token.into_string();
            format!(
                "Type: {}\n├ UTF-8: {}\n├ Hex (Memory):\n├─ Length ({}): {}\n├─ Contents ({}): {}\n├ Hex (Calldata):\n├─ Pointer ({}): {}\n├─ Length ({}): {}\n└─ Contents ({}): {}",
                Paint::red(if s.is_some() { "string" } else { "dynamic bytes" }),
                Paint::cyan(s.unwrap_or(String::from("N/A"))),
                Paint::yellow("[0x00:0x20]"),
                Paint::cyan(format!("0x{}", &hex[64..128])),
                Paint::yellow("[0x20:..]"),
                Paint::cyan(format!("0x{}", &hex[128..])),
                Paint::yellow("[0x00:0x20]"),
                Paint::cyan(format!("0x{}", &hex[..64])),
                Paint::yellow("[0x20:0x40]"),
                Paint::cyan(format!("0x{}", &hex[64..128])),
                Paint::yellow("[0x40:..]"),
                Paint::cyan(format!("0x{}", &hex[128..])),
            )
        }
        Token::FixedArray(tokens) | Token::Array(tokens) => {
            let mut out = format!("{}", Paint::red(format!("array[{}]", tokens.len())));
            out.push_str(" = [");
            for token in tokens {
                out.push_str(&"\n  ├ ");
                out.push_str(&format!("{}", format_token(token).replace('\n', "\n  ")));
                out.push_str("\n");
            }
            out.push(']');
            out
        }
        Token::Tuple(_) => {
            todo!()
        }
    }
}

// Ripped from
// [soli](https://github.com/jpopesculian/soli)
// =============================================

#[derive(Debug, Clone)]
enum Type {
    Builtin(ParamType),
    Array(Box<Type>),
    FixedArray(Box<Type>, usize),
    Map(Box<Type>, Box<Type>),
    Custom(Vec<String>),
}

impl Type {
    fn from_expression(expr: &pt::Expression) -> Option<Self> {
        Some(match expr {
            pt::Expression::Type(_, ty) => match ty {
                pt::Type::Address | pt::Type::AddressPayable | pt::Type::Payable => {
                    Type::Builtin(ParamType::Address)
                }
                pt::Type::Bool => Type::Builtin(ParamType::Bool),
                pt::Type::String => Type::Builtin(ParamType::String),
                pt::Type::Int(size) => Type::Builtin(ParamType::Int(*size as usize)),
                pt::Type::Uint(size) => Type::Builtin(ParamType::Uint(*size as usize)),
                pt::Type::Bytes(size) => Type::Builtin(ParamType::FixedBytes(*size as usize)),
                pt::Type::DynamicBytes => Type::Builtin(ParamType::Bytes),
                pt::Type::Mapping(_, left, right) => Self::Map(
                    Box::new(Type::from_expression(left)?),
                    Box::new(Type::from_expression(right)?),
                ),
                pt::Type::Function { .. } => Type::Custom(vec!["[Function]".to_string()]),
                pt::Type::Rational => Type::Custom(vec!["[Rational]".to_string()]),
            },
            pt::Expression::Variable(ident) => Type::Custom(vec![ident.name.clone()]),
            pt::Expression::ArraySubscript(_, expr, num) => {
                let num = num.as_ref().and_then(|num| {
                    if let pt::Expression::NumberLiteral(_, num, exp) = num.as_ref() {
                        let num = if num.is_empty() { 0usize } else { num.parse().ok()? };
                        let exp = if exp.is_empty() { 0u32 } else { exp.parse().ok()? };
                        Some(num * 10usize.pow(exp))
                    } else {
                        None
                    }
                });
                let ty = Type::from_expression(expr)?;
                if let Some(num) = num {
                    Self::FixedArray(Box::new(ty), num)
                } else {
                    Self::Array(Box::new(ty))
                }
            }
            pt::Expression::MemberAccess(_, expr, ident) => {
                let mut out = vec![ident.name.clone()];
                let mut cur_expr = expr;
                while let pt::Expression::MemberAccess(_, expr, ident) = cur_expr.as_ref() {
                    out.insert(0, ident.name.clone());
                    cur_expr = expr;
                }
                if let pt::Expression::Variable(ident) = cur_expr.as_ref() {
                    out.insert(0, ident.name.clone());
                }
                Type::Custom(out)
            }
            _ => return None,
        })
    }

    fn as_ethabi(&self) -> Option<ParamType> {
        match self {
            Self::Builtin(param) => Some(param.clone()),
            Self::Array(inner) => inner.as_ethabi().map(|inner| ParamType::Array(Box::new(inner))),
            Self::FixedArray(inner, size) => {
                inner.as_ethabi().map(|inner| ParamType::FixedArray(Box::new(inner), *size))
            }
            _ => None,
        }
    }
}

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
