//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use crate::prelude::{ChiselDispatcher, ChiselResult, ChiselRunner, SessionSource, SolidityHelper};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::EventParam;
use alloy_primitives::{Address, B256, U256, hex};
use eyre::{Result, WrapErr};
use foundry_compilers::Artifact;
use foundry_evm::{
    backend::Backend,
    core::evm::{BlockEnvFor, FoundryEvmNetwork, SpecFor, TxEnvFor},
    decode::decode_console_logs,
    executors::ExecutorBuilder,
    inspectors::CheatsConfig,
    traces::TraceRequirements,
};
use solar::{
    ast::{ElementaryType, LitKind, StrKind, UnOpKind},
    sema::{
        hir::{Event, Expr, ExprKind, StmtKind},
        ty::{Gcx, Ty, TyKind},
    },
};
use std::ops::ControlFlow;
use yansi::Paint;

/// Executor implementation for [SessionSource]
impl<FEN: FoundryEvmNetwork> SessionSource<FEN> {
    /// Runs the source with the [ChiselRunner]
    pub async fn execute(&mut self) -> Result<ChiselResult> {
        // Recompile the project and ensure no errors occurred.
        let output = self.build()?;

        let (bytecode, final_pc) = output.enter(|output| -> Result<_> {
            let contract = output
                .repl_contract()
                .ok_or_else(|| eyre::eyre!("failed to find REPL contract"))?;
            trace!(?contract, "REPL contract");
            let bytecode = contract
                .get_bytecode_bytes()
                .ok_or_else(|| eyre::eyre!("No bytecode found for `REPL` contract"))?;
            Ok((bytecode.into_owned(), output.final_pc(contract)?))
        })?;
        let final_pc = final_pc.unwrap_or_default();
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
                debug!(%err, "failed to build new source for inspection");
                return Ok((ControlFlow::Continue(()), None));
            }
        };

        let mut source_without_inspector = self.clone();

        // Events and tuples fails compilation due to it not being able to be encoded in
        // `inspectoor`. If that happens, try executing without the inspector.
        let (mut res, err) = match source.execute().await {
            Ok(res) => (res, None),
            Err(err) => {
                debug!(?err, %input, "execution failed");
                let should_execute = self
                    .clone_with_new_line(input.to_string())
                    .ok()
                    .and_then(|(source, do_execute)| {
                        if !do_execute {
                            return None;
                        }
                        source.build().ok().map(|output| {
                            output.enter(|output| {
                                let body = output.run_func_body();
                                let Some(last) = body.last() else { return false };
                                let StmtKind::Expr(expr) = last.kind else { return false };
                                should_continue(expr)
                            })
                        })
                    })
                    .unwrap_or(false);
                if should_execute {
                    return Ok((ControlFlow::Continue(()), None));
                }
                match source_without_inspector.execute().await {
                    Ok(res) => (res, Some(err)),
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
                let gcx = output.gcx();
                output.get_event(input).map(|eid| format_event_definition(gcx, gcx.hir.event(eid)))
            });
            if let Some(formatted_event) = formatted_event {
                return Ok((ControlFlow::Break(()), Some(formatted_event?)));
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
                ChiselDispatcher::<FEN>::show_traces(&decoder, &mut res).await?;
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
        let generated_output = source.build()?;

        // Inside the compiler closure, infer the DynSolType of the inspected expression and
        // determine whether the REPL should continue.
        let res_ty = generated_output.enter(|out| -> Option<(bool, DynSolType)> {
            let gcx = out.gcx();

            // Find the appended `bytes memory inspectoor = abi.encode(<input>);` and pull out the
            // first call argument.
            let block = out.run_func_body();
            let last = block.last()?;
            let StmtKind::DeclSingle(vid) = last.kind else { return None };
            let var = gcx.hir.variable(vid);
            let init = var.initializer?;
            let ExprKind::Call(_callee, args, _) = &init.kind else { return None };
            let inner_expr = args.exprs().next()?;

            let ty = expr_to_dyn(gcx, inner_expr)?;
            Some((should_continue(inner_expr), ty))
        });

        let Some((cont, ty)) = res_ty else {
            return Ok((ControlFlow::Continue(()), None));
        };

        // the file compiled correctly, thus the last stack item must be the memory offset of
        // the `bytes memory inspectoor` value
        let data = (|| -> Option<_> {
            let mut offset: usize = stack.last()?.try_into().ok()?;
            debug!("inspect memory @ {offset}: {}", hex::encode(memory));
            let mem_offset = memory.get(offset..offset + 32)?;
            let len: usize = U256::try_from_be_slice(mem_offset)?.try_into().ok()?;
            offset += 32;
            memory.get(offset..offset + len)
        })();
        let Some(data) = data else {
            eyre::bail!("Failed to inspect last expression: could not retrieve data from memory")
        };
        let token = ty.abi_decode(data).wrap_err("Could not decode inspected values")?;
        let c = if cont { ControlFlow::Continue(()) } else { ControlFlow::Break(()) };
        Ok((c, Some(format_token(token))))
    }

    async fn build_runner(&mut self, final_pc: usize) -> Result<ChiselRunner<FEN>> {
        let (evm_env, tx_env, fork_block) =
            self.config.evm_opts.env::<SpecFor<FEN>, BlockEnvFor<FEN>, TxEnvFor<FEN>>().await?;

        let backend = match self.config.backend.clone() {
            Some(backend) => backend,
            None => {
                let fork = self.config.evm_opts.get_fork(
                    &self.config.foundry_config,
                    evm_env.cfg_env.chain_id,
                    fork_block,
                );
                let backend = Backend::spawn(fork)?;
                self.config.backend = Some(backend.clone());
                backend
            }
        };

        let executor = ExecutorBuilder::default()
            .inspectors(|stack| {
                stack
                    .logs(self.config.foundry_config.live_logs)
                    .chisel_state(final_pc)
                    .trace_requirements(TraceRequirements::none().with_calls(true))
                    .cheatcodes(
                        CheatsConfig::new(
                            &self.config.foundry_config,
                            self.config.evm_opts.clone(),
                            None,
                            None,
                            None,
                            false,
                        )
                        .into(),
                    )
            })
            .gas_limit(self.config.evm_opts.gas_limit())
            .spec_id(self.config.foundry_config.evm_spec_id::<SpecFor<FEN>>())
            .legacy_assertions(self.config.foundry_config.legacy_assertions)
            .build(evm_env, tx_env, backend);

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
                        .chars()
                        .skip(if i.is_negative() { 64 - bit_len / 4 } else { 0 })
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
                format!("0x{i:x}").cyan(),
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

/// Formats an [`Event`] into an inspection message.
// TODO: Verbosity option
fn format_event_definition(gcx: Gcx<'_>, event: &Event<'_>) -> Result<String> {
    let event_name = event.name.as_str().to_string();
    let inputs = event
        .parameters
        .iter()
        .map(|&pid| {
            let var = gcx.hir.variable(pid);
            let name =
                var.name.map(|n| n.as_str().to_string()).unwrap_or_else(|| "<anonymous>".into());
            let kind = solar_ty_to_dyn(gcx, gcx.type_of_item(pid.into()))
                .ok_or_else(|| eyre::eyre!("Invalid type in event {event_name}"))?;
            Ok(EventParam {
                name,
                ty: kind.to_string(),
                components: vec![],
                indexed: var.indexed,
                internal_type: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let event = alloy_json_abi::Event { name: event_name, inputs, anonymous: event.anonymous };

    Ok(format!(
        "Type: {}\n├ Name: {}\n├ Signature: {:?}\n└ Selector: {:?}",
        "event".red(),
        SolidityHelper::new().highlight(&format!(
            "{}({})",
            event.name,
            event
                .inputs
                .iter()
                .map(|param| format!(
                    "{}{}{}",
                    param.ty,
                    if param.indexed { " indexed" } else { "" },
                    if param.name.is_empty() {
                        String::default()
                    } else {
                        format!(" {}", param.name)
                    },
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        event.signature().cyan(),
        event.selector().cyan(),
    ))
}

/// Converts an [`Expr`] directly to a [`DynSolType`] for ABI inspection.
fn expr_to_dyn(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<DynSolType> {
    gcx.type_of_expr(expr.id).and_then(|ty| solar_expr_ty_to_dyn(gcx, ty, expr))
}

/// Whether execution should continue after inspecting this expression.
#[inline]
fn should_continue(expr: &Expr<'_>) -> bool {
    match &expr.kind {
        // assignments and compound assignments
        ExprKind::Assign(_, _, _) => true,
        // Delete expressions.
        ExprKind::Delete(_) => true,
        // ++/-- pre/post operations
        ExprKind::Unary(op, _) => matches!(
            op.kind,
            UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
        ),
        // Array.pop()
        ExprKind::Call(callee, _, _) => match &callee.kind {
            ExprKind::Member(_, ident) => ident.as_str() == "pop",
            _ => false,
        },
        _ => false,
    }
}

/// Maps a solar [`ElementaryType`] to a [`DynSolType`].
const fn elementary_to_dyn(et: ElementaryType) -> Option<DynSolType> {
    Some(match et {
        ElementaryType::Address(_) => DynSolType::Address,
        ElementaryType::Bool => DynSolType::Bool,
        ElementaryType::String => DynSolType::String,
        ElementaryType::Bytes => DynSolType::Bytes,
        ElementaryType::Int(size) => DynSolType::Int(size.bits() as usize),
        ElementaryType::UInt(size) => DynSolType::Uint(size.bits() as usize),
        ElementaryType::FixedBytes(size) => DynSolType::FixedBytes(size.bytes() as usize),
        // Fixed-point numbers are not yet representable as DynSolType.
        ElementaryType::Fixed(_, _) | ElementaryType::UFixed(_, _) => return None,
    })
}

/// Maps a solar [`Ty`] to a [`DynSolType`].
fn solar_expr_ty_to_dyn<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>, expr: &Expr<'_>) -> Option<DynSolType> {
    // `expr` is the inspected expression inside Chisel's generated `abi.encode(...)` call. Solar
    // currently reports hex string literals as `StringLiteral`, but solc ABI-encodes
    // `hex"..."` literals as dynamic bytes in that context.
    let expr = expr.peel_parens();
    if matches!(expr.kind, ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Str(StrKind::Hex, ..)))
    {
        return Some(DynSolType::Bytes);
    }

    solar_ty_to_dyn(gcx, ty)
}

fn solar_ty_to_dyn<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> Option<DynSolType> {
    match ty.kind {
        TyKind::Elementary(et) => elementary_to_dyn(et),
        TyKind::Ref(inner, _) => solar_ty_to_dyn(gcx, inner),
        TyKind::Array(elem, n) => {
            let inner = solar_ty_to_dyn(gcx, elem)?;
            let size: usize = n.try_into().ok()?;
            Some(DynSolType::FixedArray(Box::new(inner), size))
        }
        TyKind::DynArray(elem) => {
            let inner = solar_ty_to_dyn(gcx, elem)?;
            Some(DynSolType::Array(Box::new(inner)))
        }
        TyKind::Slice(array) => solar_ty_to_dyn(gcx, array),
        TyKind::Tuple(tys) => {
            Some(DynSolType::Tuple(tys.iter().filter_map(|t| solar_ty_to_dyn(gcx, *t)).collect()))
        }
        TyKind::Mapping(_, _) => None,
        TyKind::Struct(sid) => Some(DynSolType::Tuple(
            gcx.struct_field_types(sid).iter().filter_map(|t| solar_ty_to_dyn(gcx, *t)).collect(),
        )),
        TyKind::Enum(_) => Some(DynSolType::Uint(8)),
        TyKind::Udvt(inner, _) => solar_ty_to_dyn(gcx, inner),
        TyKind::Contract(_) => Some(DynSolType::Address),
        // For a function-pointer type we return the ABI type of what the call *produces*, not a
        // representation of the pointer itself. This is intentional: chisel inspects values, so
        // the interesting type is the returned value.  A zero-return function pointer has no
        // inspectable value, so we return `None`.
        TyKind::Fn(f) => match f.returns.len() {
            0 => None,
            1 => solar_ty_to_dyn(gcx, f.returns[0]),
            _ => Some(DynSolType::Tuple(
                f.returns.iter().filter_map(|t| solar_ty_to_dyn(gcx, *t)).collect(),
            )),
        },
        TyKind::Type(inner) => solar_ty_to_dyn(gcx, inner),
        TyKind::Meta(inner) => solar_ty_to_dyn(gcx, inner),
        TyKind::IntLiteral(neg, size, _) => {
            let bits = (size.bits() as usize).max(8);
            // Round up to the nearest multiple of 8 bits, capped at 256.
            let bits = bits.div_ceil(8) * 8;
            let bits = bits.min(256);
            if neg {
                Some(DynSolType::Int(bits.max(8)))
            } else {
                Some(DynSolType::Uint(bits.max(8)))
            }
        }
        TyKind::StringLiteral(valid_utf8, _) => {
            if valid_utf8 {
                Some(DynSolType::String)
            } else {
                Some(DynSolType::Bytes)
            }
        }
        TyKind::Module(_)
        | TyKind::BuiltinModule(_)
        | TyKind::Error(_, _)
        | TyKind::Event(_, _)
        | TyKind::Err(_) => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::{error::SolcError, solc::Solc};
    use foundry_evm::core::evm::EthEvmNetwork;
    use solar::sema::Compiler;
    use std::sync::Mutex;

    type TestSessionSource = SessionSource<EthEvmNetwork>;

    #[test]
    fn test_expressions() {
        static EXPRESSIONS: &[(&str, DynSolType)] = {
            use DynSolType::*;
            &[
                // units
                // uint
                ("1 seconds", Uint(8)),
                ("1 minutes", Uint(8)),
                ("1 hours", Uint(16)),
                ("1 days", Uint(24)),
                ("1 weeks", Uint(24)),
                ("1 wei", Uint(8)),
                ("1 gwei", Uint(32)),
                ("1 ether", Uint(64)),
                // int
                ("-1 seconds", Int(8)),
                ("-1 minutes", Int(8)),
                ("-1 hours", Int(16)),
                ("-1 days", Int(24)),
                ("-1 weeks", Int(24)),
                ("-1 wei", Int(8)),
                ("-1 gwei", Int(32)),
                ("-1 ether", Int(64)),
                //
                ("true ? 1 : 0", Uint(8)),
                // misc
                //

                // ops
                // uint
                ("1 + 1", Uint(8)),
                ("1 - 1", Uint(8)),
                ("1 * 1", Uint(8)),
                ("1 / 1", Uint(8)),
                ("1 % 1", Uint(8)),
                ("1 ** 1", Uint(8)),
                ("1 | 1", Uint(8)),
                ("1 & 1", Uint(8)),
                ("1 ^ 1", Uint(8)),
                ("1 >> 1", Uint(8)),
                ("1 << 1", Uint(8)),
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
                ("!true", Bool),
                //
            ]
        };

        let source = &mut source();

        let array_expressions: &[(&str, DynSolType)] = &[
            ("[1, 2, 3]", fixed_array(DynSolType::Uint(8), 3)),
            ("[uint8(1), 2, 3]", fixed_array(DynSolType::Uint(8), 3)),
            ("[int8(1), 2, 3]", fixed_array(DynSolType::Int(8), 3)),
            ("new uint256[](3)", array(DynSolType::Uint(256))),
            ("uint256[] memory a = new uint256[](3);\na[0]", DynSolType::Uint(256)),
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
                ("1", Uint(8)),
                ("0x01", Uint(8)),
                ("int", Int(256)),
                ("int(1)", Int(256)),
                ("int(-1)", Int(256)),
                ("-1", Int(8)),
                ("-0x01", Int(8)),
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

        for n in 1..=32 {
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
                ("abi.decode(bytes(\"\"), (uint8[13]))", FixedArray(Box::new(Uint(8)), 13)),
                ("abi.decode(bytes(\"\"), (address, bytes))", Tuple(vec![Address, Bytes])),
                ("abi.decode(bytes(\"\"), (uint112, uint48))", Tuple(vec![Uint(112), Uint(48)])),
                ("abi.encode(1, 2)", Bytes),
                ("abi.encodePacked(uint256(1), uint256(2))", Bytes),
                ("abi.encodeWithSelector(bytes4(0), 1, 2)", Bytes),
                ("abi.encodeWithSignature(\"f(uint256)\", 1)", Bytes),
                //

                //
                ("bytes.concat()", Bytes),
                ("bytes.concat(bytes(\"\"))", Bytes),
                ("bytes.concat(bytes(\"\"), bytes(\"\"))", Bytes),
                ("string.concat()", String),
                ("string.concat(\"\")", String),
                ("string.concat(\"\", \"\")", String),
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
                ("blockhash(0)", FixedBytes(32)),
                ("keccak256(bytes(\"\"))", FixedBytes(32)),
                ("sha256(bytes(\"\"))", FixedBytes(32)),
                ("ripemd160(bytes(\"\"))", FixedBytes(20)),
                ("ecrecover(bytes32(0), 0, bytes32(0), bytes32(0))", Address),
                ("addmod(1, 2, 3)", Uint(256)),
                ("mulmod(1, 2, 3)", Uint(256)),
                //

                // address
                ("address(0)", Address),
                ("address(this)", Address),
                // ("super", Type::Custom("super".to_string))
                // (selfdestruct(address payable), None)
                ("address(0).balance", Uint(256)),
                ("address(0).code", Bytes),
                ("address(0).codehash", FixedBytes(32)),
                ("payable(address(0)).send(1)", Bool),
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
                ("type(Enum1).min", Uint(8)),
                ("type(Enum1).max", Uint(8)),
                // function
                ("this.run.address", Address),
                ("this.run.selector", FixedBytes(4)),
            ]
        };

        generic_type_test(&mut source(), global_variables);
    }

    #[track_caller]
    fn source() -> TestSessionSource {
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

        SessionSource::new(Default::default()).unwrap()
    }

    fn array(ty: DynSolType) -> DynSolType {
        DynSolType::Array(Box::new(ty))
    }

    fn fixed_array(ty: DynSolType, len: usize) -> DynSolType {
        DynSolType::FixedArray(Box::new(ty), len)
    }

    /// Lowers the given snippet appended to the REPL contract via solar's HIR pipeline (without
    /// invoking solc) and returns the resulting `DynSolType` of the last expression statement in
    /// the run() body.
    ///
    /// Tests bypass `SessionSource::build` (which routes through foundry-compilers + solc) so the
    /// Solar type table can be exercised directly without compiling each snippet through solc.
    fn get_type_ethabi(s: &mut TestSessionSource, input: &str, clear: bool) -> Option<DynSolType> {
        if clear {
            s.clear();
        }

        // Always declare sample types so `type(...)` tests have concrete definitions.
        *s = s.clone_with_new_line("enum Enum1 { A }".into()).unwrap().0;
        *s = s.clone_with_new_line("contract C {}".into()).unwrap().0;
        *s = s.clone_with_new_line("interface I {}".into()).unwrap().0;

        let input = format!("{};", input.trim_end().trim_end_matches(';'));
        let (new_source, _) = s.clone_with_new_line(input).unwrap();
        *s = new_source.clone();

        let src = new_source.to_repl_source();
        let mut opts = solar::interface::config::CompileOpts::default();
        opts.unstable.typeck = true;
        let sess = solar::interface::Session::builder()
            .opts(opts)
            .with_buffer_emitter(Default::default())
            .build();
        let mut compiler = Compiler::new(sess);

        compiler.enter_mut(|c| -> Option<DynSolType> {
            // Stage 1: parse, lower, and analyze (mutable access required).
            let analyzed = {
                let mut pcx = c.parse();
                let file = c
                    .sess()
                    .source_map()
                    .new_source_file(
                        std::path::PathBuf::from(new_source.file_name.clone()),
                        src.clone(),
                    )
                    .ok()?;
                pcx.add_file(file);
                pcx.parse();
                matches!(c.lower_asts(), Ok(ControlFlow::Continue(())))
                    && matches!(c.analysis(), Ok(ControlFlow::Continue(())))
            };
            if !analyzed {
                return None;
            }

            // Stage 2: walk HIR (immutable access).
            let gcx = c.gcx();
            let hir = &gcx.hir;
            let repl = hir.contracts().find(|c| c.name.as_str() == "REPL")?;
            let run_fid = repl
                .functions()
                .find(|&f| hir.function(f).name.as_ref().map(|n| n.as_str()) == Some("run"))?;
            let body = hir.function(run_fid).body?;
            let last = body.last()?;
            let expr = match last.kind {
                StmtKind::Expr(e) => e,
                _ => return None,
            };
            expr_to_dyn(gcx, expr)
        })
    }

    fn generic_type_test<'a, T, I>(s: &mut TestSessionSource, input: I)
    where
        T: AsRef<str> + std::fmt::Display + 'a,
        I: IntoIterator<Item = &'a (T, DynSolType)> + 'a,
    {
        let mut failures = Vec::new();
        for (input, expected) in input {
            let input = input.as_ref();
            let ty = get_type_ethabi(s, input, true);
            if ty.as_ref() != Some(expected) {
                failures.push(format!("{input}: got {ty:?}, expected {expected:?}"));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    fn init_tracing() {
        let _ = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }
}
