//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use std::ops::ControlFlow;

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
    hir,
    ty::{Gcx, Ty, TyKind},
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
        let output = self.build()?;

        let (bytecode, final_pc) = output.enter(|output| -> Result<_> {
            let contract = output
                .repl_contract()
                .ok_or_else(|| eyre::eyre!("failed to find REPL contract"))?;
            let bytecode = contract
                .get_bytecode_bytes()
                .ok_or_else(|| eyre::eyre!("No bytecode found for `REPL` contract"))?;
            Ok((bytecode.into_owned(), output.final_pc(contract)?))
        })?;
        dbg!(final_pc);

        let Some(final_pc) = final_pc else { return Ok(Default::default()) };

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
    pub async fn inspect(&self, input: &str) -> Result<(ControlFlow<()>, Option<String>)> {
        let line = format!("bytes memory inspectoor = abi.encode({input});");
        let mut source = match self.clone_with_new_line(line) {
            Ok((source, _)) => source,
            Err(err) => {
                debug!(%err, "failed to build new source");
                return Ok((ControlFlow::Continue(()), None));
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
                        return Ok((ControlFlow::Continue(()), None));
                    }
                }
            }
        };

        // If abi-encoding the input failed, check whether it is an event
        if let Some(err) = err {
            let output = source_without_inspector.build()?;

            let formatted_event = output.enter(|output| {
                output.get_event(input).map(|event| format_event_definition(output.gcx(), event))
            });
            if let Some(formatted_event) = formatted_event {
                return Ok((ControlFlow::Break(()), Some(formatted_event)));
            }

            // we were unable to check the event
            if self.config.foundry_config.verbosity >= 3 {
                sh_err!("Failed eval: {err}")?;
            }

            debug!(%err, %input, "failed abi encode input");
            return Ok((ControlFlow::Break(()), None));
        }
        drop(source_without_inspector);

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

        // Either the expression referred to by `input`, or the last expression,
        // which was wrapped in `abi.encode`.
        // TODO(dani): type_of_expr() of the abi.encode argument
        let resolved_input: Option<(Ty<'_>, ControlFlow<()>)> = None;
        let Some((ty, should_continue)) = resolved_input else {
            return Ok((ControlFlow::Continue(()), None));
        };
        // TODO(dani): format types even if no value?
        let output = source.build()?;
        let Some(ty) = output.enter(|output| ty_to_dyn_sol_type(output.gcx(), ty)) else {
            return Ok((ControlFlow::Continue(()), None));
        };

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
fn format_event_definition<'gcx>(gcx: Gcx<'gcx>, id: hir::EventId) -> String {
    let event = gcx.hir.event(id);
    format!(
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
    )
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

fn ty_to_dyn_sol_type(gcx: Gcx<'_>, ty: Ty<'_>) -> Option<DynSolType> {
    use DynSolType as DT;
    use TyKind as T;
    use hir::ElementaryType as ET;

    Some(match ty.kind {
        T::Elementary(t) => match t {
            ET::Address(_) => DT::Address,
            ET::Bool => DT::Bool,
            ET::String => DT::String,
            ET::Bytes => DT::Bytes,
            ET::Int(size) => DT::Int(size.bits() as usize),
            ET::UInt(size) => DT::Uint(size.bits() as usize),
            ET::FixedBytes(size) => DT::FixedBytes(size.bytes() as usize),
            _ => return None,
        },
        T::StringLiteral(..) => DT::String,
        T::IntLiteral(size) => DT::Uint(size.bits() as usize),
        T::Ref(ty, _) => ty_to_dyn_sol_type(gcx, ty)?,

        T::DynArray(elem) => DT::Array(Box::new(ty_to_dyn_sol_type(gcx, elem)?)),
        T::Array(elem, size) => {
            DT::FixedArray(Box::new(ty_to_dyn_sol_type(gcx, elem)?), size.try_into().ok()?)
        }
        T::Tuple(items) => DT::Tuple(
            items.iter().copied().map(|ty| ty_to_dyn_sol_type(gcx, ty)).collect::<Option<_>>()?,
        ),

        T::Contract(_) => DT::Address,
        T::Struct(id) => {
            if gcx.struct_recursiveness(id).is_recursive() {
                return None;
            }
            let items = gcx.struct_field_types(id);
            DT::Tuple(
                items
                    .iter()
                    .copied()
                    .map(|ty| ty_to_dyn_sol_type(gcx, ty))
                    .collect::<Option<_>>()?,
            )
        }
        T::Enum(_) => DT::Uint(8),
        T::FnPtr(_) => DT::Function,
        T::Udvt(ty, _) => ty_to_dyn_sol_type(gcx, ty)?,

        _ => return None,
    })
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
