//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use crate::prelude::{ChiselDispatcher, ChiselResult, ChiselRunner, SessionSource, SolidityHelper};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, B256, U256, hex};
use eyre::{Result, WrapErr};
use foundry_compilers::Artifact;
use foundry_evm::{
    backend::Backend, decode::decode_console_logs, executors::ExecutorBuilder,
    inspectors::CheatsConfig, traces::TraceMode,
};
use itertools::Itertools;
use solar::sema::{
    ast::Span,
    hir,
    ty::{Gcx, Ty},
};
use tracing::debug;
use yansi::Paint;

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
        let run_body = compiled.intermediate.run_func_body();

        // Record loc of first yul block return statement (if any).
        // This is used to decide which is the final statement within the `run()` method.
        // see <https://github.com/foundry-rs/foundry/issues/4617>.
        let last_yul_return_span: Option<Span> = run_body.iter().find_map(|stmt| {
            // TODO(dani): Yul is not yet lowered to HIR.
            let _ = stmt;
            /*
            if let hir::StmtKind::Assembly { block, .. } = stmt {
                if let Some(stmt) = block.last() {
                    if let pt::YulStatement::FunctionCall(yul_call) = stmt {
                        if yul_call.id.name == "return" {
                            return Some(stmt.loc())
                        }
                    }
                }
            }
            */
            None
        });

        // Find the last statement within the "run()" method and get the program
        // counter via the source map.
        let Some(last_stmt) = run_body.last() else {
            return Ok((Address::ZERO, ChiselResult::default()));
        };

        // If the final statement is some type of block (assembly, unchecked, or regular),
        // we need to find the final statement within that block. Otherwise, default to
        // the source loc of the final statement of the `run()` function's block.
        //
        // There is some code duplication within the arms due to the difference between
        // the [pt::Statement] type and the [pt::YulStatement] types.
        let mut source_span = match last_stmt.kind {
            // TODO(dani): Yul is not yet lowered to HIR.
            /*
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
                    run_body[run_body.len().saturating_sub(2)].span
                }
            }
            */
            hir::StmtKind::UncheckedBlock(stmts) | hir::StmtKind::Block(stmts) => {
                if let Some(stmt) = stmts.last() {
                    stmt.span
                } else {
                    // In the case where the block is empty, attempt to grab the statement
                    // before the block. Because we use saturating sub to get the second to
                    // last index, this can always be safely unwrapped.
                    run_body[run_body.len().saturating_sub(2)].span
                }
            }
            _ => last_stmt.span,
        };

        // Consider yul return statement as final statement (if it's loc is lower) .
        if let Some(yul_return_span) = last_yul_return_span {
            if yul_return_span.hi() < source_span.lo() {
                source_span = yul_return_span;
            }
        }

        // Map the source location of the final statement of the `run()` function to its
        // corresponding runtime program counter
        let final_pc = {
            let range =
                compiled.intermediate.sess().source_map().span_to_source(source_span).unwrap().1;
            let offset = range.start as u32;
            let length = range.len() as u32;
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
        let mut runner = self.build_runner(final_pc).await?;
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
        let mut source = match self.clone_with_new_line(line) {
            Ok((source, _)) => source,
            Err(err) => {
                debug!(%err, "failed to build new source");
                return Ok((true, None));
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
                        return Ok((true, None));
                    }
                }
            }
        };

        // If abi-encoding the input failed, check whether it is an event
        if let Some(err) = err {
            let output = source_without_inspector.build()?;
            if let Some(event) = output.intermediate.get_event(input) {
                let formatted = format_event_definition(output.intermediate.gcx(), event)?;
                return Ok((false, Some(formatted)));
            }

            // we were unable to check the event
            if self.config.foundry_config.verbosity >= 3 {
                sh_err!("Failed eval: {err}")?;
            }

            debug!(%err, %input, "failed abi encode input");
            return Ok((false, None));
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

            return Err(eyre::eyre!("Failed to inspect expression"));
        };

        // let output = source
        //     .output
        //     .as_ref()
        //     .ok_or_else(|| eyre::eyre!("Could not find generated output!"))?;

        // Either the expression referred to by `input`, or the last expression, which was wrapped
        // in `abi.encode`.
        // TODO(dani): type_of_expr() of the abi.encode argument
        let resolved_input: Option<(Ty<'_>, bool)> = None;
        let Some((ty, should_continue)) = resolved_input else { return Ok((true, None)) };
        // TODO(dani): format types even if no value?
        let Some(ty) = ty_to_dyn_sol_type(ty) else { return Ok((true, None)) };

        // the file compiled correctly, thus the last stack item must be the memory offset of
        // the `bytes memory inspectoor` value
        let mut offset = stack.last().unwrap().to::<usize>();
        let mem_offset = &memory[offset..offset + 32];
        let len = U256::try_from_be_slice(mem_offset).unwrap().to::<usize>();
        offset += 32;
        let data = &memory[offset..offset + len];
        let token = ty.abi_decode(data).wrap_err("Could not decode inspected values")?;
        Ok((should_continue, Some(format_token(token))))
    }

    async fn build_runner(&mut self, final_pc: usize) -> Result<ChiselRunner> {
        let env = self.config.evm_opts.evm_env().await?;

        let backend = match self.config.backend.take() {
            Some(backend) => backend,
            None => {
                let fork = self.config.evm_opts.get_fork(&self.config.foundry_config, env.clone());
                let backend = Backend::spawn(fork)?;
                self.config.backend = Some(backend.clone());
                backend
            }
        };

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
                    let ty = gcx.type_of_item(id.into());
                    format!(
                        "{}{}{}",
                        ty.display(gcx),
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

/// Whether execution should continue after inspecting this expression.
#[allow(dead_code)]
fn should_continue(expr: &hir::Expr<'_>) -> bool {
    match expr.kind {
        hir::ExprKind::Assign(..) => true,
        hir::ExprKind::Unary(u, _) => matches!(
            u.kind,
            hir::UnOpKind::PreInc
                | hir::UnOpKind::PreDec
                | hir::UnOpKind::PostInc
                | hir::UnOpKind::PostDec,
        ),

        // Array.pop()
        hir::ExprKind::Call(lhs, _, _) => {
            matches!(lhs.kind, hir::ExprKind::Member(_, access) if access.as_str() == "pop")
        }

        _ => false,
    }
}

fn ty_to_dyn_sol_type(ty: Ty<'_>) -> Option<DynSolType> {
    // TODO(dani)
    let _ = ty;
    None
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

#[cfg(false)] // TODO(dani): re-enable
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
                        let _ = sh_println!("found installed Solc v{v} @ {}", solc.solc.display());
                        break;
                    }
                    Err(e) => {
                        // try reinstalling
                        let _ = sh_err!("error while trying to re-install Solc v{version}: {e}");
                        let solc = Solc::blocking_install(&version.parse().unwrap());
                        if solc.map_err(SolcError::from).is_ok() {
                            *is_preinstalled = true;
                            break;
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
                let _ = sh_eprintln!("{}:{}: {}", err.loc.start(), err.loc.end(), err.message);
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
