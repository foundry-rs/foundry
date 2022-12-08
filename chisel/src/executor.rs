//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use crate::prelude::{ChiselResult, ChiselRunner, IntermediateOutput, SessionSource};
use core::fmt::Debug;
use ethers::{
    abi::{ethabi, ParamType, Token},
    types::{Address, I256, U256},
    utils::hex,
};
use ethers_solc::Artifact;
use eyre::{Result, WrapErr};
use forge::executor::{inspector::CheatsConfig, Backend, ExecutorBuilder};
use solang_parser::pt::{self, CodeLocation};
use yansi::Paint;

/// Executor implementation for [SessionSource]
impl SessionSource {
    /// Runs the source with the [ChiselRunner]
    ///
    /// ### Returns
    ///
    /// Optionally, a tuple containing the [Address] of the deployed REPL contract as well as
    /// the [ChiselResult].
    pub async fn execute(&mut self) -> Result<(Address, ChiselResult)> {
        // Recompile the project and ensure no errors occurred.
        let compiled = self.build()?;
        if let Some((_, contract)) =
            compiled.compiler_output.contracts_into_iter().find(|(name, _)| name.eq(&"REPL"))
        {
            // These *should* never panic after a successful compilation.
            let bytecode = contract.get_bytecode_bytes().expect("No bytecode for contract.");
            let deployed_bytecode =
                contract.get_deployed_bytecode_bytes().expect("No deployed bytecode for contract.");

            // Fetch the run function's body statement
            let run_func_statements = compiled.intermediate.run_func_body()?;

            // Find the last statement within the "run()" method and get the program
            // counter via the source map.
            if let Some(final_statement) = run_func_statements.last() {
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

    /// Inspect a contract element inside of the current session
    ///
    /// ### Takes
    ///
    /// A solidity snippet
    ///
    /// ### Returns
    ///
    /// If successful, a formatted inspection output.
    /// If unsuccessful but valid source, `Some(None)`
    /// If unsuccessful, `Err(e)`
    pub async fn inspect(&mut self, item: &str) -> Result<Option<String>> {
        let mut source = if let Ok((source, _)) =
            self.clone_with_new_line(format!("bytes memory inspectoor = abi.encode({item})"))
        {
            source
        } else {
            return Ok(None)
        };

        let res = if let Ok((_, res)) = source.execute().await { res } else { return Ok(None) };

        if let Some((stack, memory, _)) = &res.state {
            let generated_output = source
                .generated_output
                .as_ref()
                .ok_or(eyre::eyre!("Could not find generated output!"))?;

            // If the expression is a variable declaration within the REPL contract,
            // use its type- otherwise, attempt to infer the type.
            let ty_opt = if let Some(expr) =
                generated_output.intermediate.repl_contract_expressions.get(item)
            {
                Type::from_expression(expr)
            } else {
                self.infer_inner_expr_type(&source)
            };

            let ty = if let Some(ty) =
                ty_opt.and_then(|ty| ty.try_as_ethabi(&generated_output.intermediate))
            {
                ty
            } else {
                // Move on gracefully; This type was denied for inspection.
                return Ok(None)
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
            let mut tokens = ethabi::decode(&[ty], data).wrap_err("Could not decode ABI")?;

            tokens.pop().map_or(Err(eyre::eyre!("No tokens decoded")), |token| {
                Ok(Some(format_token(token)))
            })
        } else {
            eyre::bail!("No state present")
        }
    }

    /// Gracefully attempts to extract the type of the expression within the `abi.encode(...)`
    /// call inserted by the inspect function.
    ///
    /// ### Takes
    ///
    /// A reference to a [SessionSource]
    ///
    /// ### Returns
    ///
    /// Optionally, a [Type]
    fn infer_inner_expr_type(&mut self, source: &SessionSource) -> Option<Type> {
        if let Some(pt::Statement::VariableDefinition(
            _,
            _,
            Some(pt::Expression::FunctionCall(_, _, expressions)),
        )) =
            source.generated_output.as_ref().unwrap().intermediate.run_func_body().unwrap().last()
        {
            // We can safely unwrap the first expression because this function
            // will only be called on a session source that has just had an
            // `inspectoor` variable appended to it.
            Type::from_expression(expressions.first().unwrap())
        } else {
            None
        }
    }

    /// Prepare a runner for the Chisel REPL environment
    ///
    /// ### Takes
    ///
    /// The final statement's program counter for the [ChiselInspector]
    ///
    /// ### Returns
    ///
    /// A configured [ChiselRunner]
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
///
/// ### Takes
///
/// An owned [Token]
///
/// ### Returns
///
/// A formatted [Token] for use in inspection output.
///
/// TODO: Verbosity option
fn format_token(token: Token) -> String {
    match token {
        Token::Address(a) => {
            format!("Type: {}\n└ Data: {}", Paint::red("address"), Paint::cyan(format!("0x{a:x}")))
        }
        Token::FixedBytes(b) => {
            format!(
                "Type: {}\n└ Data: {}",
                Paint::red(format!("bytes{}", b.len())),
                Paint::cyan(format!("0x{}", hex::encode(b)))
            )
        }
        Token::Int(i) => {
            format!(
                "Type: {}\n├ Hex: {}\n└ Decimal: {}",
                Paint::red("int"),
                Paint::cyan(format!("0x{i:x}")),
                Paint::cyan(I256::from_raw(i))
            )
        }
        Token::Uint(i) => {
            format!(
                "Type: {}\n├ Hex: {}\n└ Decimal: {}",
                Paint::red("uint"),
                Paint::cyan(format!("0x{i:x}")),
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
                "Type: {}\n{}├ Hex (Memory):\n├─ Length ({}): {}\n├─ Contents ({}): {}\n├ Hex (Tuple Encoded):\n├─ Pointer ({}): {}\n├─ Length ({}): {}\n└─ Contents ({}): {}",
                Paint::red(if s.is_some() { "string" } else { "dynamic bytes" }),
                if s.is_some() {
                    format!("├ UTF-8: {}\n", Paint::cyan(s.unwrap()))
                } else {
                    String::default()
                },
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
            let mut out = format!(
                "{}({}) = {}",
                Paint::red("array"),
                Paint::yellow(format!("{}", tokens.len())),
                Paint::red('[')
            );
            for token in tokens {
                out.push_str("\n  ├ ");
                out.push_str(&format_token(token).replace('\n', "\n  "));
                out.push('\n');
            }
            out.push_str(&Paint::red(']').to_string());
            out
        }
        Token::Tuple(tokens) => {
            let mut out = format!(
                "{}({}) = {}",
                Paint::red("tuple"),
                Paint::yellow(tokens.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",")),
                Paint::red('(')
            );
            for token in tokens {
                out.push_str("\n  ├ ");
                out.push_str(&format_token(token).replace('\n', "\n  "));
                out.push('\n');
            }
            out.push_str(&Paint::red(')').to_string());
            out
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
    Custom(Vec<String>),
}

impl Type {
    /// Convert an [pt::Expression] to a [Type]
    ///
    /// ### Takes
    ///
    /// A reference to a [pt::Expression] to convert.
    ///
    /// ### Returns
    ///
    /// Optionally, an owned [Type]
    fn from_expression(expr: &pt::Expression) -> Option<Self> {
        Some(match expr {
            pt::Expression::Type(_, ty) => match ty {
                pt::Type::Address | pt::Type::AddressPayable | pt::Type::Payable => {
                    Self::Builtin(ParamType::Address)
                }
                pt::Type::Bool => Self::Builtin(ParamType::Bool),
                pt::Type::String => Self::Builtin(ParamType::String),
                pt::Type::Int(size) => Self::Builtin(ParamType::Int(*size as usize)),
                pt::Type::Uint(size) => Self::Builtin(ParamType::Uint(*size as usize)),
                pt::Type::Bytes(size) => Self::Builtin(ParamType::FixedBytes(*size as usize)),
                pt::Type::DynamicBytes => Self::Builtin(ParamType::Bytes),
                pt::Type::Mapping(_, _, right) => Self::from_expression(right)?,
                _ => return None,
            },
            pt::Expression::Variable(ident) => Self::Custom(vec![ident.name.clone()]),
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

                let ty = Self::from_expression(expr)?;
                if let Some(num) = num {
                    Self::FixedArray(Box::new(ty), num)
                } else {
                    Self::Array(Box::new(ty))
                }
            }
            pt::Expression::MemberAccess(_, expr, ident) => {
                let mut out = vec![ident.name.clone()];
                let mut cur_expr = expr;
                while let pt::Expression::FunctionCall(_, func_expr, _) = cur_expr.as_ref() {
                    if let pt::Expression::MemberAccess(_, member_expr, ident) = func_expr.as_ref()
                    {
                        out.push(ident.name.clone());
                        cur_expr = member_expr;
                    } else if let pt::Expression::Variable(ident) = func_expr.as_ref() {
                        out.push(ident.name.clone());
                        break
                    } else if let pt::Expression::Type(_, ty) = func_expr.as_ref() {
                        match ty {
                            pt::Type::Address => {
                                out.push("address".to_owned());
                                break
                            }
                            _ => break,
                        }
                        // Shouldn't ever hit here- just in case.
                        // break;
                    }
                }
                if let pt::Expression::Variable(ident) = cur_expr.as_ref() {
                    out.push(ident.name.clone());
                }
                Self::Custom(out)
            }
            pt::Expression::Parenthesis(_, inner) => Self::from_expression(inner)?,
            pt::Expression::Add(_, _, _) |
            pt::Expression::Subtract(_, _, _) |
            pt::Expression::Multiply(_, _, _) |
            pt::Expression::Divide(_, _, _) |
            pt::Expression::Modulo(_, _, _) |
            pt::Expression::Power(_, _, _) |
            pt::Expression::Complement(_, _) |
            pt::Expression::BitwiseOr(_, _, _) |
            pt::Expression::BitwiseAnd(_, _, _) |
            pt::Expression::BitwiseXor(_, _, _) |
            pt::Expression::ShiftRight(_, _, _) |
            pt::Expression::ShiftLeft(_, _, _) |
            pt::Expression::NumberLiteral(_, _, _) |
            pt::Expression::HexNumberLiteral(_, _) => Self::Builtin(ParamType::Uint(256)),
            pt::Expression::And(_, _, _) |
            pt::Expression::Or(_, _, _) |
            pt::Expression::Equal(_, _, _) |
            pt::Expression::NotEqual(_, _, _) |
            pt::Expression::Less(_, _, _) |
            pt::Expression::LessEqual(_, _, _) |
            pt::Expression::More(_, _, _) |
            pt::Expression::MoreEqual(_, _, _) |
            pt::Expression::Not(_, _) => Self::Builtin(ParamType::Bool),
            pt::Expression::StringLiteral(_) => Self::Builtin(ParamType::String),
            pt::Expression::HexLiteral(_) => Self::Builtin(ParamType::Bytes),
            pt::Expression::FunctionCall(_, outer_expr, _) => Self::from_expression(outer_expr)?,
            pt::Expression::New(_, inner) => Self::from_expression(inner)?,
            _ => return None,
        })
    }

    /// Infers a custom type's true type by recursing up the parse tree
    ///
    /// ### Takes
    /// - A reference to the [IntermediateOutput]
    /// - An array of custom types generated by the `MemberAccess` arm of [Self::from_expression]
    /// - An optional contract name. This should always be `None` when this function is first
    ///   called.
    ///
    /// ### Returns
    ///
    /// If successful, an `Ok(Some(ParamType))` variant.
    /// If gracefully failed, an `Ok(None)` variant.
    /// If failed, an `Err(e)` variant.
    fn infer_custom_type(
        intermediate: &IntermediateOutput,
        custom_type: &mut Vec<String>,
        contract_name: Option<String>,
    ) -> Result<Option<ParamType>> {
        // First, check if the contract name has been defined
        if let Some(contract_name) = contract_name {
            // Next, check if an intermediate contract exists for `contract_name`
            if let Some(intermediate_contract) =
                intermediate.intermediate_contracts.get(&contract_name)
            {
                let cur_type = custom_type.last().ok_or(eyre::eyre!(""))?;

                if let Some(func) = intermediate_contract.function_definitions.get(cur_type) {
                    // Because tuple types cannot be passed to `abi.encode`, we will only be
                    // receiving functions that have 0 or 1 return parameters
                    // here.
                    if func.returns.is_empty() {
                        eyre::bail!(
                            "This call expression does not return any values to inspect. Insert as statement."
                        )
                    } else {
                        // TODO: yuck
                        let return_ty = &func
                            .returns
                            .get(0)
                            .ok_or(eyre::eyre!("Could not find return type!"))?
                            .1
                            .as_ref()
                            .ok_or(eyre::eyre!("Could not pull expression from return type!"))?
                            .ty;

                        // If the return type is a variable (not a type expression), re-enter the
                        // recursion on the same contract for a variable / struct search. It could
                        // be a contract, struct, array, etc.
                        if let pt::Expression::Variable(pt::Identifier { loc: _, name: ident }) =
                            return_ty
                        {
                            custom_type.push(ident.clone());
                            return Self::infer_custom_type(
                                intermediate,
                                custom_type,
                                Some(contract_name),
                            )
                        }

                        // Check if our final function call alters the state. If it does, we bail so
                        // that it will be inserted normally without inspecting. If the state
                        // mutability was not expressly set, the function is inferred to alter
                        // state.
                        if let Some(pt::FunctionAttribute::Mutability(_mut)) = func
                            .attributes
                            .iter()
                            .find(|attr| matches!(attr, pt::FunctionAttribute::Mutability(_)))
                        {
                            if let pt::Mutability::Payable(_) = _mut {
                                eyre::bail!("This function mutates state. Insert as a statement.")
                            }
                        } else {
                            eyre::bail!("This function mutates state. Insert as a statement.")
                        }

                        Ok(Type::from_expression(return_ty).unwrap().try_as_ethabi(intermediate))
                    }
                } else if let Some(var_def) =
                    intermediate_contract.variable_definitions.get(cur_type)
                {
                    match &var_def.ty {
                        // If we're here, we're indexing an array within this contract. Use the
                        // inner type.
                        pt::Expression::ArraySubscript(_, expr, _) => {
                            Ok(Type::from_expression(expr).unwrap().try_as_ethabi(intermediate))
                        }
                        // Custom variable handling
                        pt::Expression::Variable(pt::Identifier { loc: _, name: ident }) => {
                            if intermediate_contract.struct_definitions.get(ident).is_some() {
                                // A struct type was found- we set the custom type to just the
                                // struct's name and re-enter the
                                // recursion one last time.
                                custom_type.clear();
                                custom_type.push(ident.to_owned());

                                Self::infer_custom_type(
                                    intermediate,
                                    custom_type,
                                    Some(contract_name),
                                )
                            } else if intermediate.intermediate_contracts.get(ident).is_some() {
                                if custom_type.len() > 1 {
                                    // There is still some recursing left to do- jump into the
                                    // contract.
                                    custom_type.pop();
                                    Self::infer_custom_type(
                                        intermediate,
                                        custom_type,
                                        Some(ident.clone()),
                                    )
                                } else {
                                    // We have no types left to recurse- return the address of the
                                    // contract.
                                    Ok(Some(ParamType::Address))
                                }
                            } else {
                                eyre::bail!("Could not infer variable type")
                            }
                        }
                        _ => Ok(Type::from_expression(&var_def.ty)
                            .unwrap()
                            .try_as_ethabi(intermediate)),
                    }
                } else if let Some(struct_def) =
                    intermediate_contract.struct_definitions.get(cur_type)
                {
                    let inner_types = struct_def
                        .fields
                        .iter()
                        .map(|var| {
                            // TODO: Check safety of these unwraps
                            Type::from_expression(&var.ty)
                                .unwrap()
                                .try_as_ethabi(intermediate)
                                .unwrap()
                        })
                        .collect::<Vec<_>>();
                    Ok(Some(ParamType::Tuple(inner_types)))
                } else {
                    eyre::bail!("Could not find function definitions for contract!")
                }
            } else {
                eyre::bail!("Could not find intermediate contract!")
            }
        } else {
            // Check if the custom type is a variable or function within the REPL contract before
            // anything. If it is, we can stop here.
            if let Ok(res) =
                Self::infer_custom_type(intermediate, custom_type, Some("REPL".to_owned()))
            {
                return Ok(res)
            }

            // Check if the first element of the custom type is a known contract. If it is, begin
            // our recursion on on that contract's definitions.
            if intermediate.intermediate_contracts.get(custom_type.last().unwrap()).is_some() {
                let removed = custom_type.pop();
                return Self::infer_custom_type(intermediate, custom_type, removed)
            }

            // If the first element of the custom type is a variable within the REPL contract,
            // that variable could be a contract itself, so we recurse back into this function
            // with a contract name set.
            //
            // We also check for an array subscript here, which could be an indexing of a mapping
            // or array. In this case, we return the type of the array.
            let var_def = intermediate.repl_contract_expressions.get(custom_type.last().unwrap());
            if let Some(var_def) = var_def {
                match &var_def {
                    pt::Expression::Variable(pt::Identifier { loc: _, name: contract_name }) => {
                        custom_type.pop();
                        return Self::infer_custom_type(
                            intermediate,
                            custom_type,
                            Some(contract_name.clone()),
                        )
                    }
                    pt::Expression::ArraySubscript(_, expr, _) => {
                        return Ok(Type::from_expression(expr).unwrap().try_as_ethabi(intermediate))
                    }
                    _ => {}
                }
            }

            // The first element of our custom type was neither a variable or a function within the
            // REPL contract, move on to globally available types gracefully.
            Ok(None)
        }
    }

    /// Attempt to convert this type into a [ParamType]
    ///
    /// ### Takes
    /// An immutable reference to an [IntermediateOutput]
    ///
    /// ### Returns
    /// Optionally, a [ParamType]
    fn try_as_ethabi(self, intermediate: &IntermediateOutput) -> Option<ParamType> {
        match self {
            Self::Builtin(param) => Some(param),
            Self::Array(inner) => match *inner {
                Self::Custom(mut types) => {
                    Self::infer_custom_type(intermediate, &mut types, None).unwrap_or(None)
                }
                _ => {
                    inner.try_as_ethabi(intermediate).map(|inner| ParamType::Array(Box::new(inner)))
                }
            },
            Self::FixedArray(inner, size) => match *inner {
                Self::Custom(mut types) => {
                    Self::infer_custom_type(intermediate, &mut types, None).unwrap_or(None)
                }
                _ => inner
                    .try_as_ethabi(intermediate)
                    .map(|inner| ParamType::FixedArray(Box::new(inner), size)),
            },
            Self::Custom(mut types) => {
                // Cover any local non-state-modifying function call expressions
                match Self::infer_custom_type(intermediate, &mut types, None) {
                    Ok(opt) => {
                        if opt.is_some() {
                            return opt
                        }
                    }
                    Err(_) => return None,
                }

                let types = types.iter().rev().collect::<Vec<&String>>();
                // Cover globally available vars / functions
                if types.len() == 1 {
                    match types[0].as_str() {
                        "gasleft" | "addmod" | "mulmod" => Some(ParamType::Uint(256)),
                        "keccak256" | "sha256" | "blockhash" => Some(ParamType::FixedBytes(32)),
                        "ripemd160" => Some(ParamType::FixedBytes(20)),
                        "ecrecover" => Some(ParamType::Address),
                        _ => None,
                    }
                } else if types.len() == 2 {
                    match types[0].as_str() {
                        "block" => match types[1].as_str() {
                            "coinbase" => Some(ParamType::Address),
                            _ => Some(ParamType::Uint(256)),
                        },
                        "msg" => match types[1].as_str() {
                            "data" => Some(ParamType::Bytes),
                            "sender" => Some(ParamType::Address),
                            "sig" => Some(ParamType::FixedBytes(4)),
                            "value" => Some(ParamType::Uint(256)),
                            _ => None,
                        },
                        "tx" => match types[1].as_str() {
                            "gasprice" => Some(ParamType::Uint(256)),
                            "origin" => Some(ParamType::Address),
                            _ => None,
                        },
                        "abi" => {
                            if types[1].starts_with("decode") {
                                // TODO: Fill value types
                                Some(ParamType::Tuple(vec![ParamType::Uint(256)]))
                            } else {
                                Some(ParamType::Bytes)
                            }
                        }
                        "address" => match types[1].as_str() {
                            "balance" => Some(ParamType::Uint(256)),
                            "code" => Some(ParamType::Bytes),
                            "codehash" => Some(ParamType::FixedBytes(32)),
                            _ => None,
                        },
                        "type" => match types[1].as_str() {
                            "name" => Some(ParamType::String),
                            "creationCode" | "runtimeCode" => Some(ParamType::Bytes),
                            "interfaceId" => Some(ParamType::FixedBytes(4)),
                            "min" | "max" => Some(ParamType::Uint(256)),
                            _ => None,
                        },
                        // TODO: Any other member access cases!
                        _ => None,
                    }
                } else {
                    None
                }
            }
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
