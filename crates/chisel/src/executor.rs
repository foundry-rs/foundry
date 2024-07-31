//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use crate::prelude::{
    ChiselDispatcher, ChiselResult, ChiselRunner, IntermediateOutput, SessionSource, SolidityHelper,
};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::EventParam;
use alloy_primitives::{hex, Address, U256};
use core::fmt::Debug;
use eyre::{Result, WrapErr};
use foundry_compilers::Artifact;
use foundry_evm::{
    backend::Backend, decode::decode_console_logs, executors::ExecutorBuilder,
    inspectors::CheatsConfig, traces::TraceMode,
};
use solang_parser::pt::{self, CodeLocation};
use std::str::FromStr;
use tracing::debug;
use yansi::Paint;

const USIZE_MAX_AS_U256: U256 = U256::from_limbs([usize::MAX as u64, 0, 0, 0]);

/// Executor implementation for [SessionSource]
impl SessionSource {
    /// Runs the source with the [ChiselRunner]
    ///
    /// ### Returns
    ///
    /// Optionally, a tuple containing the [Address] of the deployed REPL contract as well as
    /// the [ChiselResult].
    ///
    /// Returns an error if compilation fails.
    pub async fn execute(&mut self) -> Result<(Address, ChiselResult)> {
        // Recompile the project and ensure no errors occurred.
        let compiled = self.build()?;
        if let Some((_, contract)) =
            compiled.clone().compiler_output.contracts_into_iter().find(|(name, _)| name == "REPL")
        {
            // These *should* never panic after a successful compilation.
            let bytecode = contract
                .get_bytecode_bytes()
                .ok_or_else(|| eyre::eyre!("No bytecode found for `REPL` contract"))?;
            let deployed_bytecode = contract
                .get_deployed_bytecode_bytes()
                .ok_or_else(|| eyre::eyre!("No deployed bytecode found for `REPL` contract"))?;

            // Fetch the run function's body statement
            let run_func_statements = compiled.intermediate.run_func_body()?;

            // Find the last statement within the "run()" method and get the program
            // counter via the source map.
            if let Some(final_statement) = run_func_statements.last() {
                // If the final statement is some type of block (assembly, unchecked, or regular),
                // we need to find the final statement within that block. Otherwise, default to
                // the source loc of the final statement of the `run()` function's block.
                //
                // There is some code duplication within the arms due to the difference between
                // the [pt::Statement] type and the [pt::YulStatement] types.
                let source_loc = match final_statement {
                    pt::Statement::Assembly { loc: _, dialect: _, flags: _, block } => {
                        if let Some(statement) = block.statements.last() {
                            statement.loc()
                        } else {
                            // In the case where the block is empty, attempt to grab the statement
                            // before the asm block. Because we use saturating sub to get the second
                            // to last index, this can always be safely unwrapped.
                            run_func_statements
                                .get(run_func_statements.len().saturating_sub(2))
                                .unwrap()
                                .loc()
                        }
                    }
                    pt::Statement::Block { loc: _, unchecked: _, statements } => {
                        if let Some(statement) = statements.last() {
                            statement.loc()
                        } else {
                            // In the case where the block is empty, attempt to grab the statement
                            // before the block. Because we use saturating sub to get the second to
                            // last index, this can always be safely unwrapped.
                            run_func_statements
                                .get(run_func_statements.len().saturating_sub(2))
                                .unwrap()
                                .loc()
                        }
                    }
                    _ => final_statement.loc(),
                };

                // Map the source location of the final statement of the `run()` function to its
                // corresponding runtime program counter
                let final_pc = {
                    let offset = source_loc.start() as u32;
                    let length = (source_loc.end() - source_loc.start()) as u32;
                    contract
                        .get_source_map_deployed()
                        .unwrap()
                        .unwrap()
                        .into_iter()
                        .zip(InstructionIter::new(&deployed_bytecode))
                        .filter(|(s, _)| s.offset() == offset && s.length() == length)
                        .map(|(_, i)| i.pc)
                        .max()
                        .unwrap_or_default()
                };

                // Create a new runner
                let mut runner = self.prepare_runner(final_pc).await;

                // Return [ChiselResult] or bubble up error
                runner.run(bytecode.into_owned())
            } else {
                // Return a default result if no statements are present.
                Ok((Address::ZERO, ChiselResult::default()))
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
    /// If the input is valid `Ok((continue, formatted_output))` where:
    /// - `continue` is true if the input should be appended to the source
    /// - `formatted_output` is the formatted value, if any
    pub async fn inspect(&self, input: &str) -> Result<(bool, Option<String>)> {
        let line = format!("bytes memory inspectoor = abi.encode({input});");
        let mut source = match self.clone_with_new_line(line.clone()) {
            Ok((source, _)) => source,
            Err(err) => {
                debug!(%err, "failed to build new source");
                return Ok((true, None))
            }
        };

        let mut source_without_inspector = self.clone();

        // Events and tuples fails compilation due to it not being able to be encoded in
        // `inspectoor`. If that happens, try executing without the inspector.
        let (mut res, err) = match source.execute().await {
            Ok((_, res)) => (res, None),
            Err(err) => {
                debug!(?err, %input, "execution failed");
                match source_without_inspector.execute().await {
                    Ok((_, res)) => (res, Some(err)),
                    Err(_) => {
                        if self.config.foundry_config.verbosity >= 3 {
                            eprintln!("Could not inspect: {err}");
                        }
                        return Ok((true, None))
                    }
                }
            }
        };

        // If abi-encoding the input failed, check whether it is an event
        if let Some(err) = err {
            let generated_output = source_without_inspector
                .generated_output
                .as_ref()
                .ok_or_else(|| eyre::eyre!("Could not find generated output!"))?;

            let intermediate_contract = generated_output
                .intermediate
                .intermediate_contracts
                .get("REPL")
                .ok_or_else(|| eyre::eyre!("Could not find intermediate contract!"))?;

            if let Some(event_definition) = intermediate_contract.event_definitions.get(input) {
                let formatted = format_event_definition(event_definition)?;
                return Ok((false, Some(formatted)))
            }

            // we were unable to check the event
            if self.config.foundry_config.verbosity >= 3 {
                eprintln!("Failed eval: {err}");
            }

            debug!(%err, %input, "failed abi encode input");
            return Ok((false, None))
        }

        let Some((stack, memory, _)) = &res.state else {
            // Show traces and logs, if there are any, and return an error
            if let Ok(decoder) = ChiselDispatcher::decode_traces(&source.config, &mut res).await {
                ChiselDispatcher::show_traces(&decoder, &mut res).await?;
            }
            let decoded_logs = decode_console_logs(&res.logs);
            if !decoded_logs.is_empty() {
                println!("{}", "Logs:".green());
                for log in decoded_logs {
                    println!("  {log}");
                }
            }

            return Err(eyre::eyre!("Failed to inspect expression"))
        };

        let generated_output = source
            .generated_output
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Could not find generated output!"))?;

        // If the expression is a variable declaration within the REPL contract, use its type;
        // otherwise, attempt to infer the type.
        let contract_expr = generated_output
            .intermediate
            .repl_contract_expressions
            .get(input)
            .or_else(|| source.infer_inner_expr_type());

        // If the current action is a function call, we get its return type
        // otherwise it returns None
        let function_call_return_type =
            Type::get_function_return_type(contract_expr, &generated_output.intermediate);

        let (contract_expr, ty) = if let Some(function_call_return_type) = function_call_return_type
        {
            (function_call_return_type.0, function_call_return_type.1)
        } else {
            match contract_expr.and_then(|e| {
                Type::ethabi(e, Some(&generated_output.intermediate)).map(|ty| (e, ty))
            }) {
                Some(res) => res,
                // this type was denied for inspection, continue
                None => return Ok((true, None)),
            }
        };

        // the file compiled correctly, thus the last stack item must be the memory offset of
        // the `bytes memory inspectoor` value
        let mut offset = stack.last().unwrap().to::<usize>();
        let mem_offset = &memory[offset..offset + 32];
        let len = U256::try_from_be_slice(mem_offset).unwrap().to::<usize>();
        offset += 32;
        let data = &memory[offset..offset + len];
        // `tokens` is guaranteed to have the same length as the provided types
        let token =
            DynSolType::abi_decode(&ty, data).wrap_err("Could not decode inspected values")?;
        Ok((should_continue(contract_expr), Some(format_token(token))))
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
    fn infer_inner_expr_type(&self) -> Option<&pt::Expression> {
        let out = self.generated_output.as_ref()?;
        let run = out.intermediate.run_func_body().ok()?.last();
        match run {
            Some(pt::Statement::VariableDefinition(
                _,
                _,
                Some(pt::Expression::FunctionCall(_, _, args)),
            )) => {
                // We can safely unwrap the first expression because this function
                // will only be called on a session source that has just had an
                // `inspectoor` variable appended to it.
                Some(args.first().unwrap())
            }
            _ => None,
        }
    }

    /// Prepare a runner for the Chisel REPL environment
    ///
    /// ### Takes
    ///
    /// The final statement's program counter for the ChiselInspector
    ///
    /// ### Returns
    ///
    /// A configured [ChiselRunner]
    async fn prepare_runner(&mut self, final_pc: usize) -> ChiselRunner {
        let env =
            self.config.evm_opts.evm_env().await.expect("Could not instantiate fork environment");

        // Create an in-memory backend
        let backend = match self.config.backend.take() {
            Some(backend) => backend,
            None => {
                let fork = self.config.evm_opts.get_fork(&self.config.foundry_config, env.clone());
                let backend = Backend::spawn(fork);
                self.config.backend = Some(backend.clone());
                backend
            }
        };

        // Build a new executor
        let executor = ExecutorBuilder::new()
            .inspectors(|stack| {
                stack.chisel_state(final_pc).trace_mode(TraceMode::Call).cheatcodes(
                    CheatsConfig::new(
                        &self.config.foundry_config,
                        self.config.evm_opts.clone(),
                        None,
                        None,
                        Some(self.solc.version.clone()),
                    )
                    .into(),
                )
            })
            .gas_limit(self.config.evm_opts.gas_limit())
            .spec(self.config.foundry_config.evm_spec_id())
            .legacy_assertions(self.config.foundry_config.legacy_assertions)
            .build(env, backend);

        // Create a [ChiselRunner] with a default balance of [U256::MAX] and
        // the sender [Address::zero].
        ChiselRunner::new(executor, U256::MAX, Address::ZERO, self.config.calldata.clone())
    }
}

/// Formats a value into an inspection message
// TODO: Verbosity option
fn format_token(token: DynSolValue) -> String {
    match token {
        DynSolValue::Address(a) => {
            format!("Type: {}\n└ Data: {}", "address".red(), a.cyan())
        }
        DynSolValue::FixedBytes(b, byte_len) => {
            format!(
                "Type: {}\n└ Data: {}",
                format!("bytes{byte_len}").red(),
                hex::encode_prefixed(b).cyan()
            )
        }
        DynSolValue::Int(i, bit_len) => {
            format!(
                "Type: {}\n├ Hex: {}\n├ Hex (full word): {}\n└ Decimal: {}",
                format!("int{bit_len}").red(),
                format!(
                    "0x{}",
                    format!("{i:x}")
                        .char_indices()
                        .skip(64 - bit_len / 4)
                        .take(bit_len / 4)
                        .map(|(_, c)| c)
                        .collect::<String>()
                )
                .cyan(),
                format!("{i:#x}").cyan(),
                i.cyan()
            )
        }
        DynSolValue::Uint(i, bit_len) => {
            format!(
                "Type: {}\n├ Hex: {}\n├ Hex (full word): {}\n└ Decimal: {}",
                format!("uint{bit_len}").red(),
                format!(
                    "0x{}",
                    format!("{i:x}")
                        .char_indices()
                        .skip(64 - bit_len / 4)
                        .take(bit_len / 4)
                        .map(|(_, c)| c)
                        .collect::<String>()
                )
                .cyan(),
                format!("{i:#x}").cyan(),
                i.cyan()
            )
        }
        DynSolValue::Bool(b) => {
            format!("Type: {}\n└ Value: {}", "bool".red(), b.cyan())
        }
        DynSolValue::String(_) | DynSolValue::Bytes(_) => {
            let hex = hex::encode(token.abi_encode());
            let s = token.as_str();
            format!(
                "Type: {}\n{}├ Hex (Memory):\n├─ Length ({}): {}\n├─ Contents ({}): {}\n├ Hex (Tuple Encoded):\n├─ Pointer ({}): {}\n├─ Length ({}): {}\n└─ Contents ({}): {}",
                if s.is_some() { "string" } else { "dynamic bytes" }.red(),
                if let Some(s) = s {
                    format!("├ UTF-8: {}\n", s.cyan())
                } else {
                    String::default()
                },
                "[0x00:0x20]".yellow(),
                format!("0x{}", &hex[64..128]).cyan(),
                "[0x20:..]".yellow(),
                format!("0x{}", &hex[128..]).cyan(),
                "[0x00:0x20]".yellow(),
                format!("0x{}", &hex[..64]).cyan(),
                "[0x20:0x40]".yellow(),
                format!("0x{}", &hex[64..128]).cyan(),
                "[0x40:..]".yellow(),
                format!("0x{}", &hex[128..]).cyan(),
            )
        }
        DynSolValue::FixedArray(tokens) | DynSolValue::Array(tokens) => {
            let mut out = format!(
                "{}({}) = {}",
                "array".red(),
                format!("{}", tokens.len()).yellow(),
                '['.red()
            );
            for token in tokens {
                out.push_str("\n  ├ ");
                out.push_str(&format_token(token).replace('\n', "\n  "));
                out.push('\n');
            }
            out.push_str(&']'.red().to_string());
            out
        }
        DynSolValue::Tuple(tokens) => {
            let displayed_types = tokens
                .iter()
                .map(|t| t.sol_type_name().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ");
            let mut out =
                format!("{}({}) = {}", "tuple".red(), displayed_types.yellow(), '('.red());
            for token in tokens {
                out.push_str("\n  ├ ");
                out.push_str(&format_token(token).replace('\n', "\n  "));
                out.push('\n');
            }
            out.push_str(&')'.red().to_string());
            out
        }
        _ => {
            unimplemented!()
        }
    }
}

/// Formats a [pt::EventDefinition] into an inspection message
///
/// ### Takes
///
/// An borrowed [pt::EventDefinition]
///
/// ### Returns
///
/// A formatted [pt::EventDefinition] for use in inspection output.
///
/// TODO: Verbosity option
fn format_event_definition(event_definition: &pt::EventDefinition) -> Result<String> {
    let event_name = event_definition.name.as_ref().expect("Event has a name").to_string();
    let inputs = event_definition
        .fields
        .iter()
        .map(|param| {
            let name = param
                .name
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "<anonymous>".to_string());
            let kind = Type::from_expression(&param.ty)
                .and_then(Type::into_builtin)
                .ok_or_else(|| eyre::eyre!("Invalid type in event {event_name}"))?;
            Ok(EventParam {
                name,
                ty: kind.to_string(),
                components: vec![],
                indexed: param.indexed,
                internal_type: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let event =
        alloy_json_abi::Event { name: event_name, inputs, anonymous: event_definition.anonymous };

    Ok(format!(
        "Type: {}\n├ Name: {}\n├ Signature: {:?}\n└ Selector: {:?}",
        "event".red(),
        SolidityHelper::highlight(&format!(
            "{}({})",
            &event.name,
            &event
                .inputs
                .iter()
                .map(|param| format!(
                    "{}{}{}",
                    param.ty,
                    if param.indexed { " indexed" } else { "" },
                    if param.name.is_empty() {
                        String::default()
                    } else {
                        format!(" {}", &param.name)
                    },
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        event.signature().cyan(),
        event.selector().cyan(),
    ))
}

// =============================================
// Modified from
// [soli](https://github.com/jpopesculian/soli)
// =============================================

#[derive(Clone, Debug, PartialEq)]
enum Type {
    /// (type)
    Builtin(DynSolType),

    /// (type)
    Array(Box<Type>),

    /// (type, length)
    FixedArray(Box<Type>, usize),

    /// (type, index)
    ArrayIndex(Box<Type>, Option<usize>),

    /// (types)
    Tuple(Vec<Option<Type>>),

    /// (name, params, returns)
    Function(Box<Type>, Vec<Option<Type>>, Vec<Option<Type>>),

    /// (lhs, rhs)
    Access(Box<Type>, String),

    /// (types)
    Custom(Vec<String>),
}

impl Type {
    /// Convert a [pt::Expression] to a [Type]
    ///
    /// ### Takes
    ///
    /// A reference to a [pt::Expression] to convert.
    ///
    /// ### Returns
    ///
    /// Optionally, an owned [Type]
    fn from_expression(expr: &pt::Expression) -> Option<Self> {
        match expr {
            pt::Expression::Type(_, ty) => Self::from_type(ty),

            pt::Expression::Variable(ident) => Some(Self::Custom(vec![ident.name.clone()])),

            // array
            pt::Expression::ArraySubscript(_, expr, num) => {
                // if num is Some then this is either an index operation (arr[<num>])
                // or a FixedArray statement (new uint256[<num>])
                Self::from_expression(expr).and_then(|ty| {
                    let boxed = Box::new(ty);
                    let num = num.as_deref().and_then(parse_number_literal).and_then(|n| {
                        // overflow check
                        if n > USIZE_MAX_AS_U256 {
                            None
                        } else {
                            Some(n.to::<usize>())
                        }
                    });
                    match expr.as_ref() {
                        // statement
                        pt::Expression::Type(_, _) => {
                            if let Some(num) = num {
                                Some(Self::FixedArray(boxed, num))
                            } else {
                                Some(Self::Array(boxed))
                            }
                        }
                        // index
                        pt::Expression::Variable(_) => {
                            Some(Self::ArrayIndex(boxed, num))
                        }
                        _ => None
                    }
                })
            }
            pt::Expression::ArrayLiteral(_, values) => {
                values.first().and_then(Self::from_expression).map(|ty| {
                    Self::FixedArray(Box::new(ty), values.len())
                })
            }

            // tuple
            pt::Expression::List(_, params) => Some(Self::Tuple(map_parameters(params))),

            // <lhs>.<rhs>
            pt::Expression::MemberAccess(_, lhs, rhs) => {
                Self::from_expression(lhs).map(|lhs| {
                    Self::Access(Box::new(lhs), rhs.name.clone())
                })
            }

            // <inner>
            pt::Expression::Parenthesis(_, inner) |         // (<inner>)
            pt::Expression::New(_, inner) |                 // new <inner>
            pt::Expression::UnaryPlus(_, inner) |           // +<inner>
            // ops
            pt::Expression::BitwiseNot(_, inner) |          // ~<inner>
            pt::Expression::ArraySlice(_, inner, _, _) |    // <inner>[*start*:*end*]
            // assign ops
            pt::Expression::PreDecrement(_, inner) |        // --<inner>
            pt::Expression::PostDecrement(_, inner) |       // <inner>--
            pt::Expression::PreIncrement(_, inner) |        // ++<inner>
            pt::Expression::PostIncrement(_, inner) |       // <inner>++
            pt::Expression::Assign(_, inner, _) |           // <inner>   = ...
            pt::Expression::AssignAdd(_, inner, _) |        // <inner>  += ...
            pt::Expression::AssignSubtract(_, inner, _) |   // <inner>  -= ...
            pt::Expression::AssignMultiply(_, inner, _) |   // <inner>  *= ...
            pt::Expression::AssignDivide(_, inner, _) |     // <inner>  /= ...
            pt::Expression::AssignModulo(_, inner, _) |     // <inner>  %= ...
            pt::Expression::AssignAnd(_, inner, _) |        // <inner>  &= ...
            pt::Expression::AssignOr(_, inner, _) |         // <inner>  |= ...
            pt::Expression::AssignXor(_, inner, _) |        // <inner>  ^= ...
            pt::Expression::AssignShiftLeft(_, inner, _) |  // <inner> <<= ...
            pt::Expression::AssignShiftRight(_, inner, _)   // <inner> >>= ...
            => Self::from_expression(inner),

            // *condition* ? <if_true> : <if_false>
            pt::Expression::ConditionalOperator(_, _, if_true, if_false) => {
                Self::from_expression(if_true).or_else(|| Self::from_expression(if_false))
            }

            // address
            pt::Expression::AddressLiteral(_, _) => Some(Self::Builtin(DynSolType::Address)),
            pt::Expression::HexNumberLiteral(_, s, _) => {
                match s.parse::<Address>() {
                    Ok(addr) => {
                        if *s == addr.to_checksum(None) {
                            Some(Self::Builtin(DynSolType::Address))
                        } else {
                            Some(Self::Builtin(DynSolType::Uint(256)))
                        }
                    },
                    _ => {
                        Some(Self::Builtin(DynSolType::Uint(256)))
                    }
                }
            }

            // uint and int
            // invert
            pt::Expression::Negate(_, inner) => Self::from_expression(inner).map(Self::invert_int),

            // int if either operand is int
            // TODO: will need an update for Solidity v0.8.18 user defined operators:
            // https://github.com/ethereum/solidity/issues/13718#issuecomment-1341058649
            pt::Expression::Add(_, lhs, rhs) |
            pt::Expression::Subtract(_, lhs, rhs) |
            pt::Expression::Multiply(_, lhs, rhs) |
            pt::Expression::Divide(_, lhs, rhs) => {
                match (Self::ethabi(lhs, None), Self::ethabi(rhs, None)) {
                    (Some(DynSolType::Int(_)), Some(DynSolType::Int(_))) |
                    (Some(DynSolType::Int(_)), Some(DynSolType::Uint(_))) |
                    (Some(DynSolType::Uint(_)), Some(DynSolType::Int(_))) => {
                        Some(Self::Builtin(DynSolType::Int(256)))
                    }
                    _ => {
                        Some(Self::Builtin(DynSolType::Uint(256)))
                    }
                }
            }

            // always assume uint
            pt::Expression::Modulo(_, _, _) |
            pt::Expression::Power(_, _, _) |
            pt::Expression::BitwiseOr(_, _, _) |
            pt::Expression::BitwiseAnd(_, _, _) |
            pt::Expression::BitwiseXor(_, _, _) |
            pt::Expression::ShiftRight(_, _, _) |
            pt::Expression::ShiftLeft(_, _, _) |
            pt::Expression::NumberLiteral(_, _, _, _) => Some(Self::Builtin(DynSolType::Uint(256))),

            // TODO: Rational numbers
            pt::Expression::RationalNumberLiteral(_, _, _, _, _) => {
                Some(Self::Builtin(DynSolType::Uint(256)))
            }

            // bool
            pt::Expression::BoolLiteral(_, _) |
            pt::Expression::And(_, _, _) |
            pt::Expression::Or(_, _, _) |
            pt::Expression::Equal(_, _, _) |
            pt::Expression::NotEqual(_, _, _) |
            pt::Expression::Less(_, _, _) |
            pt::Expression::LessEqual(_, _, _) |
            pt::Expression::More(_, _, _) |
            pt::Expression::MoreEqual(_, _, _) |
            pt::Expression::Not(_, _) => Some(Self::Builtin(DynSolType::Bool)),

            // string
            pt::Expression::StringLiteral(_) => Some(Self::Builtin(DynSolType::String)),

            // bytes
            pt::Expression::HexLiteral(_) => Some(Self::Builtin(DynSolType::Bytes)),

            // function
            pt::Expression::FunctionCall(_, name, args) => {
                Self::from_expression(name).map(|name| {
                    let args = args.iter().map(Self::from_expression).collect();
                    Self::Function(Box::new(name), args, vec![])
                })
            }
            pt::Expression::NamedFunctionCall(_, name, args) => {
                Self::from_expression(name).map(|name| {
                    let args = args.iter().map(|arg| Self::from_expression(&arg.expr)).collect();
                    Self::Function(Box::new(name), args, vec![])
                })
            }

            // explicitly None
            pt::Expression::Delete(_, _) | pt::Expression::FunctionCallBlock(_, _, _) => None,
        }
    }

    /// Convert a [pt::Type] to a [Type]
    ///
    /// ### Takes
    ///
    /// A reference to a [pt::Type] to convert.
    ///
    /// ### Returns
    ///
    /// Optionally, an owned [Type]
    fn from_type(ty: &pt::Type) -> Option<Self> {
        let ty = match ty {
            pt::Type::Address | pt::Type::AddressPayable | pt::Type::Payable => {
                Self::Builtin(DynSolType::Address)
            }
            pt::Type::Bool => Self::Builtin(DynSolType::Bool),
            pt::Type::String => Self::Builtin(DynSolType::String),
            pt::Type::Int(size) => Self::Builtin(DynSolType::Int(*size as usize)),
            pt::Type::Uint(size) => Self::Builtin(DynSolType::Uint(*size as usize)),
            pt::Type::Bytes(size) => Self::Builtin(DynSolType::FixedBytes(*size as usize)),
            pt::Type::DynamicBytes => Self::Builtin(DynSolType::Bytes),
            pt::Type::Mapping { value, .. } => Self::from_expression(value)?,
            pt::Type::Function { params, returns, .. } => {
                let params = map_parameters(params);
                let returns = returns
                    .as_ref()
                    .map(|(returns, _)| map_parameters(returns))
                    .unwrap_or_default();
                Self::Function(
                    Box::new(Self::Custom(vec!["__fn_type__".to_string()])),
                    params,
                    returns,
                )
            }
            // TODO: Rational numbers
            pt::Type::Rational => return None,
        };
        Some(ty)
    }

    /// Handle special expressions like [global variables](https://docs.soliditylang.org/en/latest/cheatsheet.html#global-variables)
    ///
    /// See: <https://github.com/ethereum/solidity/blob/81268e336573721819e39fbb3fefbc9344ad176c/libsolidity/ast/Types.cpp#L4106>
    fn map_special(self) -> Self {
        if !matches!(self, Self::Function(_, _, _) | Self::Access(_, _) | Self::Custom(_)) {
            return self
        }

        let mut types = Vec::with_capacity(5);
        let mut args = None;
        self.recurse(&mut types, &mut args);

        let len = types.len();
        if len == 0 {
            return self
        }

        // Type members, like array, bytes etc
        #[allow(clippy::single_match)]
        match &self {
            Self::Access(inner, access) => {
                if let Some(ty) = inner.as_ref().clone().try_as_ethabi(None) {
                    // Array / bytes members
                    let ty = Self::Builtin(ty);
                    match access.as_str() {
                        "length" if ty.is_dynamic() || ty.is_array() || ty.is_fixed_bytes() => {
                            return Self::Builtin(DynSolType::Uint(256))
                        }
                        "pop" if ty.is_dynamic_array() => return ty,
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        let this = {
            let name = types.last().unwrap().as_str();
            match len {
                0 => unreachable!(),
                1 => match name {
                    "gasleft" | "addmod" | "mulmod" => Some(DynSolType::Uint(256)),
                    "keccak256" | "sha256" | "blockhash" => Some(DynSolType::FixedBytes(32)),
                    "ripemd160" => Some(DynSolType::FixedBytes(20)),
                    "ecrecover" => Some(DynSolType::Address),
                    _ => None,
                },
                2 => {
                    let access = types.first().unwrap().as_str();
                    match name {
                        "block" => match access {
                            "coinbase" => Some(DynSolType::Address),
                            "timestamp" | "difficulty" | "prevrandao" | "number" | "gaslimit" |
                            "chainid" | "basefee" | "blobbasefee" => Some(DynSolType::Uint(256)),
                            _ => None,
                        },
                        "msg" => match access {
                            "sender" => Some(DynSolType::Address),
                            "gas" => Some(DynSolType::Uint(256)),
                            "value" => Some(DynSolType::Uint(256)),
                            "data" => Some(DynSolType::Bytes),
                            "sig" => Some(DynSolType::FixedBytes(4)),
                            _ => None,
                        },
                        "tx" => match access {
                            "origin" => Some(DynSolType::Address),
                            "gasprice" => Some(DynSolType::Uint(256)),
                            _ => None,
                        },
                        "abi" => match access {
                            "decode" => {
                                // args = Some([Bytes(_), Tuple(args)])
                                // unwrapping is safe because this is first compiled by solc so
                                // it is guaranteed to be a valid call
                                let mut args = args.unwrap();
                                let last = args.pop().unwrap();
                                match last {
                                    Some(ty) => {
                                        return match ty {
                                            Self::Tuple(_) => ty,
                                            ty => Self::Tuple(vec![Some(ty)]),
                                        }
                                    }
                                    None => None,
                                }
                            }
                            s if s.starts_with("encode") => Some(DynSolType::Bytes),
                            _ => None,
                        },
                        "address" => match access {
                            "balance" => Some(DynSolType::Uint(256)),
                            "code" => Some(DynSolType::Bytes),
                            "codehash" => Some(DynSolType::FixedBytes(32)),
                            "send" => Some(DynSolType::Bool),
                            _ => None,
                        },
                        "type" => match access {
                            "name" => Some(DynSolType::String),
                            "creationCode" | "runtimeCode" => Some(DynSolType::Bytes),
                            "interfaceId" => Some(DynSolType::FixedBytes(4)),
                            "min" | "max" => Some(
                                // Either a builtin or an enum
                                (|| args?.pop()??.into_builtin())()
                                    .unwrap_or(DynSolType::Uint(256)),
                            ),
                            _ => None,
                        },
                        "string" => match access {
                            "concat" => Some(DynSolType::String),
                            _ => None,
                        },
                        "bytes" => match access {
                            "concat" => Some(DynSolType::Bytes),
                            _ => None,
                        },
                        _ => None,
                    }
                }
                _ => None,
            }
        };

        this.map(Self::Builtin).unwrap_or_else(|| match types.last().unwrap().as_str() {
            "this" | "super" => Self::Custom(types),
            _ => match self {
                Self::Custom(_) | Self::Access(_, _) => Self::Custom(types),
                Self::Function(_, _, _) => self,
                _ => unreachable!(),
            },
        })
    }

    /// Recurses over itself, appending all the idents and function arguments in the order that they
    /// are found
    fn recurse(&self, types: &mut Vec<String>, args: &mut Option<Vec<Option<Self>>>) {
        match self {
            Self::Builtin(ty) => types.push(ty.to_string()),
            Self::Custom(tys) => types.extend(tys.clone()),
            Self::Access(expr, name) => {
                types.push(name.clone());
                expr.recurse(types, args);
            }
            Self::Function(fn_name, fn_args, _fn_ret) => {
                if args.is_none() && !fn_args.is_empty() {
                    *args = Some(fn_args.clone());
                }
                fn_name.recurse(types, args);
            }
            _ => {}
        }
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
    /// If successful, an `Ok(Some(DynSolType))` variant.
    /// If gracefully failed, an `Ok(None)` variant.
    /// If failed, an `Err(e)` variant.
    fn infer_custom_type(
        intermediate: &IntermediateOutput,
        custom_type: &mut Vec<String>,
        contract_name: Option<String>,
    ) -> Result<Option<DynSolType>> {
        if let Some("this") | Some("super") = custom_type.last().map(String::as_str) {
            custom_type.pop();
        }
        if custom_type.is_empty() {
            return Ok(None)
        }

        // If a contract exists with the given name, check its definitions for a match.
        // Otherwise look in the `run`
        if let Some(contract_name) = contract_name {
            let intermediate_contract = intermediate
                .intermediate_contracts
                .get(&contract_name)
                .ok_or_else(|| eyre::eyre!("Could not find intermediate contract!"))?;

            let cur_type = custom_type.last().unwrap();
            if let Some(func) = intermediate_contract.function_definitions.get(cur_type) {
                // Check if the custom type is a function pointer member access
                if let res @ Some(_) = func_members(func, custom_type) {
                    return Ok(res)
                }

                // Because tuple types cannot be passed to `abi.encode`, we will only be
                // receiving functions that have 0 or 1 return parameters here.
                if func.returns.is_empty() {
                    eyre::bail!(
                        "This call expression does not return any values to inspect. Insert as statement."
                    )
                }

                // Empty return types check is done above
                let (_, param) = func.returns.first().unwrap();
                // Return type should always be present
                let return_ty = &param.as_ref().unwrap().ty;

                // If the return type is a variable (not a type expression), re-enter the recursion
                // on the same contract for a variable / struct search. It could be a contract,
                // struct, array, etc.
                if let pt::Expression::Variable(ident) = return_ty {
                    custom_type.push(ident.name.clone());
                    return Self::infer_custom_type(intermediate, custom_type, Some(contract_name))
                }

                // Check if our final function call alters the state. If it does, we bail so that it
                // will be inserted normally without inspecting. If the state mutability was not
                // expressly set, the function is inferred to alter state.
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

                Ok(Self::ethabi(return_ty, Some(intermediate)))
            } else if let Some(var) = intermediate_contract.variable_definitions.get(cur_type) {
                Self::infer_var_expr(&var.ty, Some(intermediate), custom_type)
            } else if let Some(strukt) = intermediate_contract.struct_definitions.get(cur_type) {
                let inner_types = strukt
                    .fields
                    .iter()
                    .map(|var| {
                        Self::ethabi(&var.ty, Some(intermediate))
                            .ok_or_else(|| eyre::eyre!("Struct `{cur_type}` has invalid fields"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Some(DynSolType::Tuple(inner_types)))
            } else {
                eyre::bail!("Could not find any definition in contract \"{contract_name}\" for type: {custom_type:?}")
            }
        } else {
            // Check if the custom type is a variable or function within the REPL contract before
            // anything. If it is, we can stop here.
            if let Ok(res) = Self::infer_custom_type(intermediate, custom_type, Some("REPL".into()))
            {
                return Ok(res)
            }

            // Check if the first element of the custom type is a known contract. If it is, begin
            // our recursion on on that contract's definitions.
            let name = custom_type.last().unwrap();
            let contract = intermediate.intermediate_contracts.get(name);
            if contract.is_some() {
                let contract_name = custom_type.pop();
                return Self::infer_custom_type(intermediate, custom_type, contract_name)
            }

            // See [`Type::infer_var_expr`]
            let name = custom_type.last().unwrap();
            if let Some(expr) = intermediate.repl_contract_expressions.get(name) {
                return Self::infer_var_expr(expr, Some(intermediate), custom_type)
            }

            // The first element of our custom type was neither a variable or a function within the
            // REPL contract, move on to globally available types gracefully.
            Ok(None)
        }
    }

    /// Infers the type from a variable's type
    fn infer_var_expr(
        expr: &pt::Expression,
        intermediate: Option<&IntermediateOutput>,
        custom_type: &mut Vec<String>,
    ) -> Result<Option<DynSolType>> {
        // Resolve local (in `run` function) or global (in the `REPL` or other contract) variable
        let res = match &expr {
            // Custom variable handling
            pt::Expression::Variable(ident) => {
                let name = &ident.name;

                if let Some(intermediate) = intermediate {
                    // expression in `run`
                    if let Some(expr) = intermediate.repl_contract_expressions.get(name) {
                        Self::infer_var_expr(expr, Some(intermediate), custom_type)
                    } else if intermediate.intermediate_contracts.contains_key(name) {
                        if custom_type.len() > 1 {
                            // There is still some recursing left to do: jump into the contract.
                            custom_type.pop();
                            Self::infer_custom_type(intermediate, custom_type, Some(name.clone()))
                        } else {
                            // We have no types left to recurse: return the address of the contract.
                            Ok(Some(DynSolType::Address))
                        }
                    } else {
                        Err(eyre::eyre!("Could not infer variable type"))
                    }
                } else {
                    Ok(None)
                }
            }
            ty => Ok(Self::ethabi(ty, intermediate)),
        };
        // re-run everything with the resolved variable in case we're accessing a builtin member
        // for example array or bytes length etc
        match res {
            Ok(Some(ty)) => {
                let box_ty = Box::new(Self::Builtin(ty.clone()));
                let access = Self::Access(box_ty, custom_type.drain(..).next().unwrap_or_default());
                if let Some(mapped) = access.map_special().try_as_ethabi(intermediate) {
                    Ok(Some(mapped))
                } else {
                    Ok(Some(ty))
                }
            }
            res => res,
        }
    }

    /// Attempt to convert this type into a [DynSolType]
    ///
    /// ### Takes
    /// An immutable reference to an [IntermediateOutput]
    ///
    /// ### Returns
    /// Optionally, a [DynSolType]
    fn try_as_ethabi(self, intermediate: Option<&IntermediateOutput>) -> Option<DynSolType> {
        match self {
            Self::Builtin(ty) => Some(ty),
            Self::Tuple(types) => Some(DynSolType::Tuple(types_to_parameters(types, intermediate))),
            Self::Array(inner) => match *inner {
                ty @ Self::Custom(_) => ty.try_as_ethabi(intermediate),
                _ => inner
                    .try_as_ethabi(intermediate)
                    .map(|inner| DynSolType::Array(Box::new(inner))),
            },
            Self::FixedArray(inner, size) => match *inner {
                ty @ Self::Custom(_) => ty.try_as_ethabi(intermediate),
                _ => inner
                    .try_as_ethabi(intermediate)
                    .map(|inner| DynSolType::FixedArray(Box::new(inner), size)),
            },
            ty @ Self::ArrayIndex(_, _) => ty.into_array_index(intermediate),
            Self::Function(ty, _, _) => ty.try_as_ethabi(intermediate),
            // should have been mapped to `Custom` in previous steps
            Self::Access(_, _) => None,
            Self::Custom(mut types) => {
                // Cover any local non-state-modifying function call expressions
                intermediate.and_then(|intermediate| {
                    Self::infer_custom_type(intermediate, &mut types, None).ok().flatten()
                })
            }
        }
    }

    /// Equivalent to `Type::from_expression` + `Type::map_special` + `Type::try_as_ethabi`
    fn ethabi(
        expr: &pt::Expression,
        intermediate: Option<&IntermediateOutput>,
    ) -> Option<DynSolType> {
        Self::from_expression(expr)
            .map(Self::map_special)
            .and_then(|ty| ty.try_as_ethabi(intermediate))
    }

    /// Get the return type of a function call expression.
    fn get_function_return_type<'a>(
        contract_expr: Option<&'a pt::Expression>,
        intermediate: &IntermediateOutput,
    ) -> Option<(&'a pt::Expression, DynSolType)> {
        let function_call = match contract_expr? {
            pt::Expression::FunctionCall(_, function_call, _) => function_call,
            _ => return None,
        };
        let (contract_name, function_name) = match function_call.as_ref() {
            pt::Expression::MemberAccess(_, contract_name, function_name) => {
                (contract_name, function_name)
            }
            _ => return None,
        };
        let contract_name = match contract_name.as_ref() {
            pt::Expression::Variable(contract_name) => contract_name.to_owned(),
            _ => return None,
        };

        let pt::Expression::Variable(contract_name) =
            intermediate.repl_contract_expressions.get(&contract_name.name)?
        else {
            return None
        };

        let contract = intermediate
            .intermediate_contracts
            .get(&contract_name.name)?
            .function_definitions
            .get(&function_name.name)?;
        let return_parameter = contract.as_ref().returns.first()?.to_owned().1?;
        Self::ethabi(&return_parameter.ty, Some(intermediate)).map(|p| (contract_expr.unwrap(), p))
    }

    /// Inverts Int to Uint and viceversa.
    fn invert_int(self) -> Self {
        match self {
            Self::Builtin(DynSolType::Uint(n)) => Self::Builtin(DynSolType::Int(n)),
            Self::Builtin(DynSolType::Int(n)) => Self::Builtin(DynSolType::Uint(n)),
            x => x,
        }
    }

    /// Returns the `DynSolType` contained by `Type::Builtin`
    #[inline]
    fn into_builtin(self) -> Option<DynSolType> {
        match self {
            Self::Builtin(ty) => Some(ty),
            _ => None,
        }
    }

    /// Returns the resulting `DynSolType` of indexing self
    fn into_array_index(self, intermediate: Option<&IntermediateOutput>) -> Option<DynSolType> {
        match self {
            Self::Array(inner) | Self::FixedArray(inner, _) | Self::ArrayIndex(inner, _) => {
                match inner.try_as_ethabi(intermediate) {
                    Some(DynSolType::Array(inner)) | Some(DynSolType::FixedArray(inner, _)) => {
                        Some(*inner)
                    }
                    Some(DynSolType::Bytes) |
                    Some(DynSolType::String) |
                    Some(DynSolType::FixedBytes(_)) => Some(DynSolType::FixedBytes(1)),
                    ty => ty,
                }
            }
            _ => None,
        }
    }

    /// Returns whether this type is dynamic
    #[inline]
    fn is_dynamic(&self) -> bool {
        match self {
            // TODO: Note, this is not entirely correct. Fixed arrays of non-dynamic types are
            // not dynamic, nor are tuples of non-dynamic types.
            Self::Builtin(DynSolType::Bytes | DynSolType::String | DynSolType::Array(_)) => true,
            Self::Array(_) => true,
            _ => false,
        }
    }

    /// Returns whether this type is an array
    #[inline]
    fn is_array(&self) -> bool {
        matches!(
            self,
            Self::Array(_) |
                Self::FixedArray(_, _) |
                Self::Builtin(DynSolType::Array(_)) |
                Self::Builtin(DynSolType::FixedArray(_, _))
        )
    }

    /// Returns whether this type is a dynamic array (can call push, pop)
    #[inline]
    fn is_dynamic_array(&self) -> bool {
        matches!(self, Self::Array(_) | Self::Builtin(DynSolType::Array(_)))
    }

    fn is_fixed_bytes(&self) -> bool {
        matches!(self, Self::Builtin(DynSolType::FixedBytes(_)))
    }
}

/// Returns Some if the custom type is a function member access
///
/// Ref: <https://docs.soliditylang.org/en/latest/types.html#function-types>
#[inline]
fn func_members(func: &pt::FunctionDefinition, custom_type: &[String]) -> Option<DynSolType> {
    if !matches!(func.ty, pt::FunctionTy::Function) {
        return None
    }

    let vis = func.attributes.iter().find_map(|attr| match attr {
        pt::FunctionAttribute::Visibility(vis) => Some(vis),
        _ => None,
    });
    match vis {
        Some(pt::Visibility::External(_)) | Some(pt::Visibility::Public(_)) => {
            match custom_type.first().unwrap().as_str() {
                "address" => Some(DynSolType::Address),
                "selector" => Some(DynSolType::FixedBytes(4)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether execution should continue after inspecting this expression
#[inline]
fn should_continue(expr: &pt::Expression) -> bool {
    #[allow(clippy::match_like_matches_macro)]
    match expr {
        // assignments
        pt::Expression::PreDecrement(_, _) |       // --<inner>
        pt::Expression::PostDecrement(_, _) |      // <inner>--
        pt::Expression::PreIncrement(_, _) |       // ++<inner>
        pt::Expression::PostIncrement(_, _) |      // <inner>++
        pt::Expression::Assign(_, _, _) |          // <inner>   = ...
        pt::Expression::AssignAdd(_, _, _) |       // <inner>  += ...
        pt::Expression::AssignSubtract(_, _, _) |  // <inner>  -= ...
        pt::Expression::AssignMultiply(_, _, _) |  // <inner>  *= ...
        pt::Expression::AssignDivide(_, _, _) |    // <inner>  /= ...
        pt::Expression::AssignModulo(_, _, _) |    // <inner>  %= ...
        pt::Expression::AssignAnd(_, _, _) |       // <inner>  &= ...
        pt::Expression::AssignOr(_, _, _) |        // <inner>  |= ...
        pt::Expression::AssignXor(_, _, _) |       // <inner>  ^= ...
        pt::Expression::AssignShiftLeft(_, _, _) | // <inner> <<= ...
        pt::Expression::AssignShiftRight(_, _, _)  // <inner> >>= ...
        => {
            true
        }

        // Array.pop()
        pt::Expression::FunctionCall(_, lhs, _) => {
            match lhs.as_ref() {
                pt::Expression::MemberAccess(_, _inner, access) => access.name == "pop",
                _ => false
            }
        }

        _ => false
    }
}

fn map_parameters(params: &[(pt::Loc, Option<pt::Parameter>)]) -> Vec<Option<Type>> {
    params
        .iter()
        .map(|(_, param)| param.as_ref().and_then(|param| Type::from_expression(&param.ty)))
        .collect()
}

fn types_to_parameters(
    types: Vec<Option<Type>>,
    intermediate: Option<&IntermediateOutput>,
) -> Vec<DynSolType> {
    types.into_iter().filter_map(|ty| ty.and_then(|ty| ty.try_as_ethabi(intermediate))).collect()
}

fn parse_number_literal(expr: &pt::Expression) -> Option<U256> {
    match expr {
        pt::Expression::NumberLiteral(_, num, exp, unit) => {
            let num = U256::from_str(num).unwrap_or(U256::ZERO);
            let exp = exp.parse().unwrap_or(0u32);
            if exp > 77 {
                None
            } else {
                let exp = U256::from(10usize.pow(exp));
                let unit_mul = unit_multiplier(unit).ok()?;
                Some(num * exp * unit_mul)
            }
        }
        pt::Expression::HexNumberLiteral(_, num, unit) => {
            let unit_mul = unit_multiplier(unit).ok()?;
            num.parse::<U256>().map(|num| num * unit_mul).ok()
        }
        // TODO: Rational numbers
        pt::Expression::RationalNumberLiteral(..) => None,
        _ => None,
    }
}

#[inline]
fn unit_multiplier(unit: &Option<pt::Identifier>) -> Result<U256> {
    if let Some(unit) = unit {
        let mul = match unit.name.as_str() {
            "seconds" => 1,
            "minutes" => 60,
            "hours" => 60 * 60,
            "days" => 60 * 60 * 24,
            "weeks" => 60 * 60 * 24 * 7,
            "wei" => 1,
            "gwei" => 10_usize.pow(9),
            "ether" => 10_usize.pow(18),
            other => eyre::bail!("unknown unit: {other}"),
        };
        Ok(U256::from(mul))
    } else {
        Ok(U256::from(1))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::{error::SolcError, solc::Solc};
    use semver::Version;
    use std::sync::Mutex;

    #[test]
    fn test_const() {
        assert_eq!(USIZE_MAX_AS_U256.to::<u64>(), usize::MAX as u64);
        assert_eq!(USIZE_MAX_AS_U256.to::<u64>(), usize::MAX as u64);
    }

    #[test]
    fn test_expressions() {
        static EXPRESSIONS: &[(&str, DynSolType)] = {
            use DynSolType::*;
            &[
                // units
                // uint
                ("1 seconds", Uint(256)),
                ("1 minutes", Uint(256)),
                ("1 hours", Uint(256)),
                ("1 days", Uint(256)),
                ("1 weeks", Uint(256)),
                ("1 wei", Uint(256)),
                ("1 gwei", Uint(256)),
                ("1 ether", Uint(256)),
                // int
                ("-1 seconds", Int(256)),
                ("-1 minutes", Int(256)),
                ("-1 hours", Int(256)),
                ("-1 days", Int(256)),
                ("-1 weeks", Int(256)),
                ("-1 wei", Int(256)),
                ("-1 gwei", Int(256)),
                ("-1 ether", Int(256)),
                //
                ("true ? 1 : 0", Uint(256)),
                ("true ? -1 : 0", Int(256)),
                // misc
                //

                // ops
                // uint
                ("1 + 1", Uint(256)),
                ("1 - 1", Uint(256)),
                ("1 * 1", Uint(256)),
                ("1 / 1", Uint(256)),
                ("1 % 1", Uint(256)),
                ("1 ** 1", Uint(256)),
                ("1 | 1", Uint(256)),
                ("1 & 1", Uint(256)),
                ("1 ^ 1", Uint(256)),
                ("1 >> 1", Uint(256)),
                ("1 << 1", Uint(256)),
                // int
                ("int(1) + 1", Int(256)),
                ("int(1) - 1", Int(256)),
                ("int(1) * 1", Int(256)),
                ("int(1) / 1", Int(256)),
                ("1 + int(1)", Int(256)),
                ("1 - int(1)", Int(256)),
                ("1 * int(1)", Int(256)),
                ("1 / int(1)", Int(256)),
                //

                // assign
                ("uint256 a; a--", Uint(256)),
                ("uint256 a; --a", Uint(256)),
                ("uint256 a; a++", Uint(256)),
                ("uint256 a; ++a", Uint(256)),
                ("uint256 a; a   = 1", Uint(256)),
                ("uint256 a; a  += 1", Uint(256)),
                ("uint256 a; a  -= 1", Uint(256)),
                ("uint256 a; a  *= 1", Uint(256)),
                ("uint256 a; a  /= 1", Uint(256)),
                ("uint256 a; a  %= 1", Uint(256)),
                ("uint256 a; a  &= 1", Uint(256)),
                ("uint256 a; a  |= 1", Uint(256)),
                ("uint256 a; a  ^= 1", Uint(256)),
                ("uint256 a; a <<= 1", Uint(256)),
                ("uint256 a; a >>= 1", Uint(256)),
                //

                // bool
                ("true && true", Bool),
                ("true || true", Bool),
                ("true == true", Bool),
                ("true != true", Bool),
                ("true < true", Bool),
                ("true <= true", Bool),
                ("true > true", Bool),
                ("true >= true", Bool),
                ("!true", Bool),
                //
            ]
        };

        let source = &mut source();

        let array_expressions: &[(&str, DynSolType)] = &[
            ("[1, 2, 3]", fixed_array(DynSolType::Uint(256), 3)),
            ("[uint8(1), 2, 3]", fixed_array(DynSolType::Uint(8), 3)),
            ("[int8(1), 2, 3]", fixed_array(DynSolType::Int(8), 3)),
            ("new uint256[](3)", array(DynSolType::Uint(256))),
            ("uint256[] memory a = new uint256[](3);\na[0]", DynSolType::Uint(256)),
            ("uint256[] memory a = new uint256[](3);\na[0:3]", array(DynSolType::Uint(256))),
        ];
        generic_type_test(source, array_expressions);
        generic_type_test(source, EXPRESSIONS);
    }

    #[test]
    fn test_types() {
        static TYPES: &[(&str, DynSolType)] = {
            use DynSolType::*;
            &[
                // bool
                ("bool", Bool),
                ("true", Bool),
                ("false", Bool),
                //

                // int and uint
                ("uint", Uint(256)),
                ("uint(1)", Uint(256)),
                ("1", Uint(256)),
                ("0x01", Uint(256)),
                ("int", Int(256)),
                ("int(1)", Int(256)),
                ("int(-1)", Int(256)),
                ("-1", Int(256)),
                ("-0x01", Int(256)),
                //

                // address
                ("address", Address),
                ("address(0)", Address),
                ("0x690B9A9E9aa1C9dB991C7721a92d351Db4FaC990", Address),
                ("payable(0)", Address),
                ("payable(address(0))", Address),
                //

                // string
                ("string", String),
                ("string(\"hello world\")", String),
                ("\"hello world\"", String),
                ("unicode\"hello world 😀\"", String),
                //

                // bytes
                ("bytes", Bytes),
                ("bytes(\"hello world\")", Bytes),
                ("bytes(unicode\"hello world 😀\")", Bytes),
                ("hex\"68656c6c6f20776f726c64\"", Bytes),
                //
            ]
        };

        let mut types: Vec<(String, DynSolType)> = Vec::with_capacity(96 + 32 + 100);
        for (n, b) in (8..=256).step_by(8).zip(1..=32) {
            types.push((format!("uint{n}(0)"), DynSolType::Uint(n)));
            types.push((format!("int{n}(0)"), DynSolType::Int(n)));
            types.push((format!("bytes{b}(0x00)"), DynSolType::FixedBytes(b)));
        }

        for n in 0..=32 {
            types.push((
                format!("uint256[{n}]"),
                DynSolType::FixedArray(Box::new(DynSolType::Uint(256)), n),
            ));
        }

        generic_type_test(&mut source(), TYPES);
        generic_type_test(&mut source(), &types);
    }

    #[test]
    fn test_global_vars() {
        init_tracing();

        // https://docs.soliditylang.org/en/latest/cheatsheet.html#global-variables
        let global_variables = {
            use DynSolType::*;
            &[
                // abi
                ("abi.decode(bytes, (uint8[13]))", Tuple(vec![FixedArray(Box::new(Uint(8)), 13)])),
                ("abi.decode(bytes, (address, bytes))", Tuple(vec![Address, Bytes])),
                ("abi.decode(bytes, (uint112, uint48))", Tuple(vec![Uint(112), Uint(48)])),
                ("abi.encode(_, _)", Bytes),
                ("abi.encodePacked(_, _)", Bytes),
                ("abi.encodeWithSelector(bytes4, _, _)", Bytes),
                ("abi.encodeCall(function(), (_, _))", Bytes),
                ("abi.encodeWithSignature(string, _, _)", Bytes),
                //

                //
                ("bytes.concat()", Bytes),
                ("bytes.concat(_)", Bytes),
                ("bytes.concat(_, _)", Bytes),
                ("string.concat()", String),
                ("string.concat(_)", String),
                ("string.concat(_, _)", String),
                //

                // block
                ("block.basefee", Uint(256)),
                ("block.chainid", Uint(256)),
                ("block.coinbase", Address),
                ("block.difficulty", Uint(256)),
                ("block.gaslimit", Uint(256)),
                ("block.number", Uint(256)),
                ("block.timestamp", Uint(256)),
                //

                // tx
                ("gasleft()", Uint(256)),
                ("msg.data", Bytes),
                ("msg.sender", Address),
                ("msg.sig", FixedBytes(4)),
                ("msg.value", Uint(256)),
                ("tx.gasprice", Uint(256)),
                ("tx.origin", Address),
                //

                // assertions
                // assert(bool)
                // require(bool)
                // revert()
                // revert(string)
                //

                //
                ("blockhash(uint)", FixedBytes(32)),
                ("keccak256(bytes)", FixedBytes(32)),
                ("sha256(bytes)", FixedBytes(32)),
                ("ripemd160(bytes)", FixedBytes(20)),
                ("ecrecover(bytes32, uint8, bytes32, bytes32)", Address),
                ("addmod(uint, uint, uint)", Uint(256)),
                ("mulmod(uint, uint, uint)", Uint(256)),
                //

                // address
                ("address(_)", Address),
                ("address(this)", Address),
                // ("super", Type::Custom("super".to_string))
                // (selfdestruct(address payable), None)
                ("address.balance", Uint(256)),
                ("address.code", Bytes),
                ("address.codehash", FixedBytes(32)),
                ("address.send(uint256)", Bool),
                // (address.transfer(uint256), None)
                //

                // type
                ("type(C).name", String),
                ("type(C).creationCode", Bytes),
                ("type(C).runtimeCode", Bytes),
                ("type(I).interfaceId", FixedBytes(4)),
                ("type(uint256).min", Uint(256)),
                ("type(int128).min", Int(128)),
                ("type(int256).min", Int(256)),
                ("type(uint256).max", Uint(256)),
                ("type(int128).max", Int(128)),
                ("type(int256).max", Int(256)),
                ("type(Enum1).min", Uint(256)),
                ("type(Enum1).max", Uint(256)),
                // function
                ("this.run.address", Address),
                ("this.run.selector", FixedBytes(4)),
            ]
        };

        generic_type_test(&mut source(), global_variables);
    }

    #[track_caller]
    fn source() -> SessionSource {
        // synchronize solc install
        static PRE_INSTALL_SOLC_LOCK: Mutex<bool> = Mutex::new(false);

        // on some CI targets installing results in weird malformed solc files, we try installing it
        // multiple times
        let version = "0.8.20";
        for _ in 0..3 {
            let mut is_preinstalled = PRE_INSTALL_SOLC_LOCK.lock().unwrap();
            if !*is_preinstalled {
                let solc = Solc::find_or_install(&version.parse().unwrap())
                    .map(|solc| (solc.version.clone(), solc));
                match solc {
                    Ok((v, solc)) => {
                        // successfully installed
                        eprintln!("found installed Solc v{v} @ {}", solc.solc.display());
                        break
                    }
                    Err(e) => {
                        // try reinstalling
                        eprintln!("error while trying to re-install Solc v{version}: {e}");
                        let solc = Solc::blocking_install(&version.parse().unwrap());
                        if solc.map_err(SolcError::from).is_ok() {
                            *is_preinstalled = true;
                            break
                        }
                    }
                }
            }
        }

        let solc = Solc::find_or_install(&Version::new(0, 8, 19)).expect("could not install solc");
        SessionSource::new(solc, Default::default())
    }

    fn array(ty: DynSolType) -> DynSolType {
        DynSolType::Array(Box::new(ty))
    }

    fn fixed_array(ty: DynSolType, len: usize) -> DynSolType {
        DynSolType::FixedArray(Box::new(ty), len)
    }

    fn parse(s: &mut SessionSource, input: &str, clear: bool) -> IntermediateOutput {
        if clear {
            s.drain_run();
            s.drain_top_level_code();
            s.drain_global_code();
        }

        *s = s.clone_with_new_line("enum Enum1 { A }".into()).unwrap().0;

        let input = format!("{};", input.trim_end().trim_end_matches(';'));
        let (mut _s, _) = s.clone_with_new_line(input).unwrap();
        *s = _s.clone();
        let s = &mut _s;

        if let Err(e) = s.parse() {
            for err in e {
                eprintln!("{}:{}: {}", err.loc.start(), err.loc.end(), err.message);
            }
            let source = s.to_repl_source();
            panic!("could not parse input:\n{source}")
        }
        s.generate_intermediate_output().expect("could not generate intermediate output")
    }

    fn expr(stmts: &[pt::Statement]) -> pt::Expression {
        match stmts.last().expect("no statements") {
            pt::Statement::Expression(_, e) => e.clone(),
            s => panic!("Not an expression: {s:?}"),
        }
    }

    fn get_type(
        s: &mut SessionSource,
        input: &str,
        clear: bool,
    ) -> (Option<Type>, IntermediateOutput) {
        let intermediate = parse(s, input, clear);
        let run_func_body = intermediate.run_func_body().expect("no run func body");
        let expr = expr(run_func_body);
        (Type::from_expression(&expr).map(Type::map_special), intermediate)
    }

    fn get_type_ethabi(s: &mut SessionSource, input: &str, clear: bool) -> Option<DynSolType> {
        let (ty, intermediate) = get_type(s, input, clear);
        ty.and_then(|ty| ty.try_as_ethabi(Some(&intermediate)))
    }

    fn generic_type_test<'a, T, I>(s: &mut SessionSource, input: I)
    where
        T: AsRef<str> + std::fmt::Display + 'a,
        I: IntoIterator<Item = &'a (T, DynSolType)> + 'a,
    {
        for (input, expected) in input.into_iter() {
            let input = input.as_ref();
            let ty = get_type_ethabi(s, input, true);
            assert_eq!(ty.as_ref(), Some(expected), "\n{input}");
        }
    }

    fn init_tracing() {
        let _ = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }
}
