//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use crate::prelude::{
    ChiselDispatcher, ChiselResult, ChiselRunner, IntermediateOutput, SessionSource, SolidityHelper,
};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::EventParam;
use alloy_primitives::{hex, Address, B256, U256};
use core::fmt::Debug;
use eyre::{Result, WrapErr};
use foundry_compilers::Artifact;
use foundry_evm::{
    backend::Backend, decode::decode_console_logs, executors::ExecutorBuilder,
    inspectors::CheatsConfig, traces::TraceMode,
};
use itertools::Itertools;
use solang_parser::pt::{self, CodeLocation};
use solar_sema::{
    hir,
    ty::{self, Gcx},
};
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

        let (_, contract) = compiled
            .compiler
            .contracts_iter()
            .find(|&(name, _)| name == "REPL")
            .ok_or_else(|| eyre::eyre!("failed to find REPL contract"))?;

        // These *should* never panic after a successful compilation.
        let bytecode = contract
            .get_bytecode_bytes()
            .ok_or_else(|| eyre::eyre!("No bytecode found for `REPL` contract"))?;
        let deployed_bytecode = contract
            .get_deployed_bytecode_bytes()
            .ok_or_else(|| eyre::eyre!("No deployed bytecode found for `REPL` contract"))?;

        // Fetch the run function's body statement
        let run_func_statements = compiled.intermediate.run_func_body()?;

        // Record loc of first yul block return statement (if any).
        // This is used to decide which is the final statement within the `run()` method.
        // see <https://github.com/foundry-rs/foundry/issues/4617>.
        let last_yul_return = run_func_statements.iter().find_map(|statement| {
            if let pt::Statement::Assembly { loc: _, dialect: _, flags: _, block } = statement {
                if let Some(statement) = block.statements.last() {
                    if let pt::YulStatement::FunctionCall(yul_call) = statement {
                        if yul_call.id.name == "return" {
                            return Some(statement.loc())
                        }
                    }
                }
            }
            None
        });

        // Find the last statement within the "run()" method and get the program
        // counter via the source map.
        let Some(last_stmt) = run_func_statements.last() else {
            return Ok((Address::ZERO, ChiselResult::default()));
        };

        // If the final statement is some type of block (assembly, unchecked, or regular),
        // we need to find the final statement within that block. Otherwise, default to
        // the source loc of the final statement of the `run()` function's block.
        //
        // There is some code duplication within the arms due to the difference between
        // the [pt::Statement] type and the [pt::YulStatement] types.
        let mut source_loc = match last_stmt {
            pt::Statement::Assembly { loc: _, dialect: _, flags: _, block } => {
                // Select last non variable declaration statement, see <https://github.com/foundry-rs/foundry/issues/4938>.
                let last_statement = block.statements.iter().rev().find(|statement| {
                    !matches!(statement, pt::YulStatement::VariableDeclaration(_, _, _))
                });
                if let Some(statement) = last_statement {
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
            _ => last_stmt.loc(),
        };

        // Consider yul return statement as final statement (if it's loc is lower) .
        if let Some(yul_return) = last_yul_return {
            if yul_return.end() < source_loc.start() {
                source_loc = yul_return;
            }
        }

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

        let bytecode = bytecode.into_owned();
        let mut runner = self.prepare_runner(final_pc).await?;
        runner.run(bytecode)
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
                            sh_err!("Could not inspect: {err}")?;
                        }
                        return Ok((true, None))
                    }
                }
            }
        };

        // If abi-encoding the input failed, check whether it is an event
        if let Some(err) = err {
            let output = source_without_inspector
                .output
                .as_ref()
                .ok_or_else(|| eyre::eyre!("Could not find generated output!"))?;
            if let Some(event) = output.intermediate.get_event(input) {
                let formatted = format_event_definition(output.intermediate.gcx(), event)?;
                return Ok((false, Some(formatted)))
            }

            // we were unable to check the event
            if self.config.foundry_config.verbosity >= 3 {
                sh_err!("Failed eval: {err}")?;
            }

            debug!(%err, %input, "failed abi encode input");
            return Ok((false, None))
        }

        let Some((stack, memory)) = &res.state else {
            // Show traces and logs, if there are any, and return an error
            if let Ok(decoder) = ChiselDispatcher::decode_traces(&source.config, &mut res).await {
                ChiselDispatcher::show_traces(&decoder, &mut res).await?;
            }
            let decoded_logs = decode_console_logs(&res.logs);
            if !decoded_logs.is_empty() {
                sh_println!("{}", "Logs:".green())?;
                for log in decoded_logs {
                    sh_println!("  {log}")?;
                }
            }

            return Err(eyre::eyre!("Failed to inspect expression"))
        };

        let output = source
            .output
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Could not find generated output!"))?;

        // If the expression is a variable declaration within the REPL contract, use its type;
        // otherwise, attempt to infer the type.
        let contract_expr = output
            .intermediate
            .get_var(input)
            .map(|id| {
                let gcx = output.intermediate.gcx();
                (gcx.type_of_item(id.into()), gcx.hir.variable(id).initializer)
            })
            .or_else(|| {
                // TODO(dani): type_of_expr() of the abi.encode argument
                // source.infer_inner_expr_type()
                let expr = source
                    .infer_inner_expr_type()
                    .and_then(|e| output.intermediate.get_expr(e));
                (None)
            });

        // If the current action is a function call, we get its return type
        // otherwise it returns None
        let function_call_return_type =
            Type::get_function_return_type(contract_expr, &output.intermediate);

        let (contract_expr, ty) = if let Some(r) = function_call_return_type {
            r
        } else {
            match contract_expr
                .and_then(|e| Type::ethabi(e, Some(&output.intermediate)).map(|ty| (e, ty)))
            {
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
        // TODO(dani)
        None
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
    async fn prepare_runner(&mut self, final_pc: usize) -> Result<ChiselRunner> {
        let env =
            self.config.evm_opts.evm_env().await.expect("Could not instantiate fork environment");

        // Create an in-memory backend
        let backend = match self.config.backend.take() {
            Some(backend) => backend,
            None => {
                let fork = self.config.evm_opts.get_fork(&self.config.foundry_config, env.clone());
                let backend = Backend::spawn(fork)?;
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
                    )
                    .into(),
                )
            })
            .gas_limit(self.config.evm_opts.gas_limit())
            .spec_id(self.config.foundry_config.evm_spec_id())
            .legacy_assertions(self.config.foundry_config.legacy_assertions)
            .build(env, backend);

        // Create a [ChiselRunner] with a default balance of [U256::MAX] and
        // the sender [Address::zero].
        Ok(ChiselRunner::new(executor, U256::MAX, Address::ZERO, self.config.calldata.clone()))
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
                hex::encode_prefixed(B256::from(i)).cyan(),
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
                hex::encode_prefixed(B256::from(i)).cyan(),
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

// TODO: Verbosity option
fn format_event_definition<'gcx>(gcx: Gcx<'gcx>, id: hir::EventId) -> Result<String> {
    let event = gcx.hir.event(id);
    Ok(format!(
        "Type: {}\n├ Name: {}\n├ Signature: {:?}\n└ Selector: {:?}",
        "event".red(),
        SolidityHelper::new().highlight(&format!(
            "{}({})",
            event.name,
            event
                .parameters
                .iter()
                .map(|&id| {
                    let param = gcx.hir.variable(id);
                    format!(
                        "{}{}{}",
                        // param.ty,
                        "<ty>", // TODO(dani): param.ty.display(gcx),
                        if param.indexed { " indexed" } else { "" },
                        if let Some(name) = param.name {
                            format!(" {name}")
                        } else {
                            String::new()
                        },
                    )
                })
                .format(", ")
        )),
        gcx.item_signature(id.into()).cyan(),
        gcx.event_selector(id).cyan(),
    ))
}

/// Whether execution should continue after inspecting this expression
fn should_continue(expr: &pt::Expression) -> bool {
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

impl Iterator for InstructionIter<'_> {
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
