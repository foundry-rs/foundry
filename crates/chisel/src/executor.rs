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
    backend::Backend, decode::decode_console_logs, executors::ExecutorBuilder,
    inspectors::CheatsConfig, traces::TraceMode,
};
use solar::{
    ast::{BinOpKind, ElementaryType, FunctionKind, LitKind, StateMutability, StrKind, UnOpKind},
    interface::Symbol,
    sema::{
        hir::{
            ContractId, Event, Expr, ExprKind, Function, ItemId, Res, StmtKind, Type as HirType,
            TypeKind, Visibility,
        },
        ty::{Gcx, Ty, TyKind},
    },
};
use std::ops::ControlFlow;
use yansi::Paint;

/// Executor implementation for [SessionSource]
impl SessionSource {
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
        let generated_output = source.build()?;

        // Inside the compiler closure, infer the DynSolType of the inspected expression and
        // determine whether the REPL should continue.
        let res_ty = generated_output.enter(|out| -> Option<(bool, DynSolType)> {
            let gcx = out.gcx();

            // Try direct lookup of `input` as a named variable in the REPL contract.
            if let Some(direct_ty) = lookup_named_variable_type(gcx, input) {
                return Some((false, direct_ty));
            }

            // Otherwise, find the appended `bytes memory inspectoor = abi.encode(<input>);`
            // and pull out the first call argument.
            let block = out.run_func_body();
            let last = block.last()?;
            let StmtKind::DeclSingle(vid) = last.kind else { return None };
            let var = gcx.hir.variable(vid);
            let init = var.initializer?;
            let ExprKind::Call(_callee, args, _) = &init.kind else { return None };
            let inner_expr = args.exprs().next()?;

            // If the call is `func()` returning a single value, prefer the function return type.
            if let Some(ty) = get_function_return_type(gcx, inner_expr) {
                return Some((should_continue(inner_expr), ty));
            }

            let ty = expr_to_dyn(gcx, inner_expr, true)?;
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

    async fn build_runner(&mut self, final_pc: usize) -> Result<ChiselRunner> {
        let (evm_env, tx_env, fork_block) = self.config.evm_opts.env().await?;

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
                    .trace_mode(TraceMode::Call)
                    .cheatcodes(
                        CheatsConfig::new(
                            &self.config.foundry_config,
                            self.config.evm_opts.clone(),
                            None,
                            None,
                            None,
                        )
                        .into(),
                    )
            })
            .gas_limit(self.config.evm_opts.gas_limit())
            .spec_id(self.config.foundry_config.evm_spec_id())
            .legacy_assertions(self.config.foundry_config.legacy_assertions)
            .build(evm_env, tx_env, backend);

        Ok(ChiselRunner::new(executor, U256::MAX, Address::ZERO, self.config.calldata.clone()))
    }
}

/// Looks up `name` as a named variable in the REPL contract (state variables or run() locals)
/// and returns its type as a [`DynSolType`].
///
/// Only top-level statements of `run()` are scanned. Variables declared inside nested blocks
/// (`if`, `for`, `while`, `unchecked`, etc.) are not visible here; the caller falls back to
/// the `inspectoor`-based path for those cases.
fn lookup_named_variable_type(gcx: Gcx<'_>, name: &str) -> Option<DynSolType> {
    let hir = &gcx.hir;
    let repl = hir.contracts().find(|c| c.name.as_str() == "REPL")?;

    // State variables.
    for vid in repl.variables() {
        let var = hir.variable(vid);
        if var.name.map(|n| n.as_str() == name).unwrap_or(false) {
            return solar_ty_to_dyn(gcx, gcx.type_of_item(vid.into()));
        }
    }

    // Locals declared in run().
    let run_fid = repl
        .functions()
        .find(|&f| hir.function(f).name.as_ref().map(|n| n.as_str()) == Some("run"))?;
    let body = hir.function(run_fid).body?;
    for stmt in body.stmts {
        match stmt.kind {
            StmtKind::DeclSingle(vid) => {
                let var = hir.variable(vid);
                if var.name.map(|n| n.as_str() == name).unwrap_or(false) {
                    return solar_ty_to_dyn(gcx, gcx.type_of_item(vid.into()));
                }
            }
            StmtKind::DeclMulti(vids, _) => {
                for vid in vids.iter().flatten() {
                    let var = hir.variable(*vid);
                    if var.name.map(|n| n.as_str() == name).unwrap_or(false) {
                        return solar_ty_to_dyn(gcx, gcx.type_of_item((*vid).into()));
                    }
                }
            }
            _ => {}
        }
    }
    None
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

// =============================================
// Modified from
// [soli](https://github.com/jpopesculian/soli)
// =============================================

/// Converts an [`Expr`] directly to a [`DynSolType`] for ABI inspection.
///
/// `lookup` controls whether user-defined type names are resolved via the HIR.
fn expr_to_dyn(gcx: Gcx<'_>, expr: &Expr<'_>, lookup: bool) -> Option<DynSolType> {
    match &expr.kind {
        // Elementary type expression: `uint256`, `address`, etc.
        ExprKind::Type(ty) => hir_ty_to_dyn(gcx, ty),

        // `type(T)`: only meaningful as the lhs of a member access.
        ExprKind::TypeCall(_) => None,

        // Literals.
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Address(_) => Some(DynSolType::Address),
            LitKind::Bool(_) => Some(DynSolType::Bool),
            LitKind::Str(kind, _, _) => match kind {
                StrKind::Hex => Some(DynSolType::Bytes),
                StrKind::Str | StrKind::Unicode => Some(DynSolType::String),
            },
            LitKind::Number(_) | LitKind::Rational(_) => Some(DynSolType::Uint(256)),
            LitKind::Err(_) => None,
        },

        // Resolved identifier: `foo`.
        ExprKind::Ident(reses) => {
            let res = reses.first()?;
            match *res {
                Res::Item(ItemId::Variable(vid)) => {
                    solar_ty_to_dyn(gcx, gcx.type_of_item(vid.into()))
                }
                Res::Item(ItemId::Struct(sid)) => {
                    // Struct reference used as a constructor produces a tuple of field types.
                    Some(DynSolType::Tuple(
                        gcx.struct_field_types(sid)
                            .iter()
                            .filter_map(|&t| solar_ty_to_dyn(gcx, t))
                            .collect(),
                    ))
                }
                // Other items and builtins: handled by enclosing Call/Member expressions.
                _ => None,
            }
        }

        // Index/access: `arr[i]`, `MyType[]`, `MyType[N]`.
        ExprKind::Index(base, idx) => {
            let base_ty = expr_to_dyn(gcx, base, lookup)?;
            let num =
                idx.and_then(|e| parse_number_literal(e)).and_then(|n| usize::try_from(n).ok());
            match &base.kind {
                // Type-level indexing builds an array type expression.
                ExprKind::Type(_) | ExprKind::TypeCall(_) => {
                    if let Some(n) = num {
                        Some(DynSolType::FixedArray(Box::new(base_ty), n))
                    } else {
                        Some(DynSolType::Array(Box::new(base_ty)))
                    }
                }
                // Runtime indexing returns the element type.
                _ => match base_ty {
                    DynSolType::Array(inner) | DynSolType::FixedArray(inner, _) => Some(*inner),
                    DynSolType::Bytes | DynSolType::String | DynSolType::FixedBytes(_) => {
                        Some(DynSolType::FixedBytes(1))
                    }
                    other => Some(other),
                },
            }
        }

        // Slice: same type as the base.
        ExprKind::Slice(base, _, _) => expr_to_dyn(gcx, base, lookup),

        // Array literal `[a, b, c]`.
        ExprKind::Array(values) => values
            .first()
            .and_then(|e| expr_to_dyn(gcx, e, lookup))
            .map(|ty| DynSolType::FixedArray(Box::new(ty), values.len())),

        // Tuple expression `(a, b, c)`.
        ExprKind::Tuple(items) => Some(DynSolType::Tuple(
            items.iter().filter_map(|opt| opt.and_then(|e| expr_to_dyn(gcx, e, lookup))).collect(),
        )),

        // Member access `lhs.member`.
        ExprKind::Member(_, _) => resolve_member(gcx, expr, lookup),

        // Function/constructor call.
        ExprKind::Call(_, _, _) => resolve_call(gcx, expr, lookup),

        // `new T`: produces a value of type T.
        ExprKind::New(ty) => hir_ty_to_dyn(gcx, ty),

        // `payable(addr)`.
        ExprKind::Payable(_) => Some(DynSolType::Address),

        // Ternary: prefer truthy branch's type, fall back to else branch.
        ExprKind::Ternary(_, t, e) => {
            expr_to_dyn(gcx, t, lookup).or_else(|| expr_to_dyn(gcx, e, lookup))
        }

        // Delete has no return type.
        ExprKind::Delete(_) => None,

        // Unary operations.
        ExprKind::Unary(op, inner) => match op.kind {
            UnOpKind::Neg => expr_to_dyn(gcx, inner, lookup).map(|ty| match ty {
                DynSolType::Uint(n) => DynSolType::Int(n),
                DynSolType::Int(n) => DynSolType::Uint(n),
                x => x,
            }),
            UnOpKind::Not => Some(DynSolType::Bool),
            UnOpKind::BitNot
            | UnOpKind::PreInc
            | UnOpKind::PreDec
            | UnOpKind::PostInc
            | UnOpKind::PostDec => expr_to_dyn(gcx, inner, lookup),
        },

        // Binary operations.
        ExprKind::Binary(lhs, op, rhs) => match op.kind {
            BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
            | BinOpKind::And
            | BinOpKind::Or => Some(DynSolType::Bool),
            BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul | BinOpKind::Div => {
                match (expr_to_dyn(gcx, lhs, false), expr_to_dyn(gcx, rhs, false)) {
                    (Some(DynSolType::Int(_) | DynSolType::Uint(_)), Some(DynSolType::Int(_)))
                    | (Some(DynSolType::Int(_)), Some(DynSolType::Uint(_))) => {
                        Some(DynSolType::Int(256))
                    }
                    _ => Some(DynSolType::Uint(256)),
                }
            }
            BinOpKind::Rem
            | BinOpKind::Pow
            | BinOpKind::BitAnd
            | BinOpKind::BitOr
            | BinOpKind::BitXor
            | BinOpKind::Shl
            | BinOpKind::Shr
            | BinOpKind::Sar => Some(DynSolType::Uint(256)),
        },

        // Assignments: type of the lhs.
        ExprKind::Assign(lhs, _, _) => expr_to_dyn(gcx, lhs, lookup),

        ExprKind::Err(_) => None,
    }
}

/// Converts a [`HirType`] to a [`DynSolType`].
fn hir_ty_to_dyn(gcx: Gcx<'_>, ty: &HirType<'_>) -> Option<DynSolType> {
    match &ty.kind {
        TypeKind::Elementary(et) => elementary_to_dyn(*et),
        TypeKind::Array(arr) => {
            let elem = hir_ty_to_dyn(gcx, &arr.element)?;
            if let Some(size) = arr.size {
                let n = parse_number_literal(size).and_then(|n| usize::try_from(n).ok());
                if let Some(n) = n {
                    Some(DynSolType::FixedArray(Box::new(elem), n))
                } else {
                    Some(DynSolType::Array(Box::new(elem)))
                }
            } else {
                Some(DynSolType::Array(Box::new(elem)))
            }
        }
        TypeKind::Function(f) => match f.returns.len() {
            0 => None,
            1 => {
                let var = gcx.hir.variable(f.returns[0]);
                hir_ty_to_dyn(gcx, &var.ty)
            }
            _ => Some(DynSolType::Tuple(
                f.returns
                    .iter()
                    .filter_map(|&pid| hir_ty_to_dyn(gcx, &gcx.hir.variable(pid).ty))
                    .collect(),
            )),
        },
        TypeKind::Mapping(m) => hir_ty_to_dyn(gcx, &m.value),
        TypeKind::Custom(item) => solar_ty_to_dyn(gcx, gcx.type_of_item(*item)),
        TypeKind::Err(_) => None,
    }
}

/// Resolves a member-access expression (`lhs.member`) to its [`DynSolType`].
///
/// `expr` must be `ExprKind::Member`.
fn resolve_member(gcx: Gcx<'_>, expr: &Expr<'_>, lookup: bool) -> Option<DynSolType> {
    let ExprKind::Member(lhs, ident) = &expr.kind else { return None };
    let member = ident.name;

    // `type(T).member` — type introspection.
    if let ExprKind::TypeCall(ty) = &lhs.kind {
        return match member.as_str() {
            "name" => Some(DynSolType::String),
            "creationCode" | "runtimeCode" => Some(DynSolType::Bytes),
            "interfaceId" => Some(DynSolType::FixedBytes(4)),
            // Only valid for integer types; custom types (enums) fall back to Uint(256).
            "min" | "max" => match &ty.kind {
                TypeKind::Elementary(et) => elementary_to_dyn(*et),
                _ => Some(DynSolType::Uint(256)),
            },
            _ => None,
        };
    }

    // Built-in namespace identifier: `block.timestamp`, `msg.sender`, `abi.encode`, etc.
    if let ExprKind::Ident(reses) = &lhs.kind
        && let Some(Res::Builtin(b)) = reses.first()
        && let Some(ty) = builtin_member(b.name().as_str(), member.as_str())
    {
        return Some(ty);
    }

    // Elementary type used as a namespace: `address.balance`, `bytes.concat`, etc.
    if let ExprKind::Type(ty) = &lhs.kind
        && let TypeKind::Elementary(et) = &ty.kind
    {
        return match et {
            ElementaryType::Address(_) => match member.as_str() {
                "balance" => Some(DynSolType::Uint(256)),
                "code" => Some(DynSolType::Bytes),
                "codehash" => Some(DynSolType::FixedBytes(32)),
                "send" => Some(DynSolType::Bool),
                _ => None,
            },
            ElementaryType::Bytes => match member.as_str() {
                "concat" => Some(DynSolType::Bytes),
                _ => None,
            },
            ElementaryType::String => match member.as_str() {
                "concat" => Some(DynSolType::String),
                _ => None,
            },
            _ => None,
        };
    }

    // Members on a resolved DynSolType (`.length`, `.pop`, `.selector`, `.address`).
    if let Some(lhs_ty) = expr_to_dyn(gcx, lhs, lookup)
        && let Some(ty) = dyn_member(&lhs_ty, member.as_str())
    {
        return Some(ty);
    }

    // HIR lookup for user-defined type members.
    if lookup && let Some(mut chain) = expr_name_chain(gcx, lhs) {
        chain.insert(0, member);
        return infer_custom_type(gcx, &mut chain, None).ok().flatten();
    }

    None
}

/// Returns the type of `builtin_ns.member` for built-in global namespaces.
fn builtin_member(builtin: &str, member: &str) -> Option<DynSolType> {
    match builtin {
        "block" => match member {
            "coinbase" => Some(DynSolType::Address),
            "timestamp" | "difficulty" | "prevrandao" | "number" | "gaslimit" | "chainid"
            | "basefee" | "blobbasefee" => Some(DynSolType::Uint(256)),
            _ => None,
        },
        "msg" => match member {
            "sender" => Some(DynSolType::Address),
            "gas" | "value" => Some(DynSolType::Uint(256)),
            "data" => Some(DynSolType::Bytes),
            "sig" => Some(DynSolType::FixedBytes(4)),
            _ => None,
        },
        "tx" => match member {
            "origin" => Some(DynSolType::Address),
            "gasprice" => Some(DynSolType::Uint(256)),
            _ => None,
        },
        "address" => match member {
            "balance" => Some(DynSolType::Uint(256)),
            "code" => Some(DynSolType::Bytes),
            "codehash" => Some(DynSolType::FixedBytes(32)),
            "send" => Some(DynSolType::Bool),
            _ => None,
        },
        _ => None,
    }
}

/// Returns the type of `ty.member` for a known [`DynSolType`].
fn dyn_member(ty: &DynSolType, member: &str) -> Option<DynSolType> {
    match member {
        "length" => match ty {
            DynSolType::Array(_)
            | DynSolType::FixedArray(_, _)
            | DynSolType::Bytes
            | DynSolType::String
            | DynSolType::FixedBytes(_) => Some(DynSolType::Uint(256)),
            _ => None,
        },
        "pop" => match ty {
            DynSolType::Array(inner) => Some(*inner.clone()),
            _ => None,
        },
        // Address members.
        "balance" => match ty {
            DynSolType::Address => Some(DynSolType::Uint(256)),
            _ => None,
        },
        "code" => match ty {
            DynSolType::Address => Some(DynSolType::Bytes),
            _ => None,
        },
        "codehash" => match ty {
            DynSolType::Address => Some(DynSolType::FixedBytes(32)),
            _ => None,
        },
        "send" => match ty {
            DynSolType::Address => Some(DynSolType::Bool),
            _ => None,
        },
        // External function members.
        "selector" => Some(DynSolType::FixedBytes(4)),
        "address" => Some(DynSolType::Address),
        _ => None,
    }
}

/// Resolves a call expression to its return [`DynSolType`].
///
/// `expr` must be `ExprKind::Call`.
fn resolve_call(gcx: Gcx<'_>, expr: &Expr<'_>, lookup: bool) -> Option<DynSolType> {
    let ExprKind::Call(callee, args, _named) = &expr.kind else { return None };

    // Type cast: `uint256(x)`, `address(y)`, etc.
    if let ExprKind::Type(ty) = &callee.kind {
        return hir_ty_to_dyn(gcx, ty);
    }

    // Member call: `ns.method(...)`.
    if let ExprKind::Member(lhs, method) = &callee.kind
        && let ExprKind::Ident(reses) = &lhs.kind
        && let Some(Res::Builtin(b)) = reses.first()
    {
        match b.name().as_str() {
            "abi" => {
                return match method.as_str() {
                    "decode" => {
                        let last = args.exprs().last()?;
                        match expr_to_dyn(gcx, last, false)? {
                            DynSolType::Tuple(tys) => Some(DynSolType::Tuple(tys)),
                            ty => Some(DynSolType::Tuple(vec![ty])),
                        }
                    }
                    s if s.starts_with("encode") => Some(DynSolType::Bytes),
                    _ => None,
                };
            }
            "string" if method.as_str() == "concat" => return Some(DynSolType::String),
            "bytes" if method.as_str() == "concat" => return Some(DynSolType::Bytes),
            _ => {}
        }
    }

    // Simple identifier call: built-in global functions and HIR function calls.
    if let ExprKind::Ident(reses) = &callee.kind {
        match reses.first() {
            Some(Res::Builtin(b)) => {
                return match b.name().as_str() {
                    "gasleft" | "addmod" | "mulmod" => Some(DynSolType::Uint(256)),
                    "keccak256" | "sha256" | "blockhash" => Some(DynSolType::FixedBytes(32)),
                    "ripemd160" => Some(DynSolType::FixedBytes(20)),
                    "ecrecover" => Some(DynSolType::Address),
                    _ => None,
                };
            }
            Some(Res::Item(ItemId::Function(fid))) if lookup => {
                let func = gcx.hir.function(*fid);
                if !matches!(func.state_mutability, StateMutability::View | StateMutability::Pure) {
                    return None;
                }
                let ret_id = *func.returns.first()?;
                return solar_ty_to_dyn(gcx, gcx.type_of_item(ret_id.into()));
            }
            _ => {}
        }
    }

    // Fall back to the callee's resolved type.
    expr_to_dyn(gcx, callee, lookup)
}

/// Extracts a name chain from a member-access expression tree for HIR lookup.
///
/// The chain is ordered outermost-first so `a.b.c` produces `["c", "b", "a"]` with the root
/// identifier at the back. This matches the convention expected by [`infer_custom_type`].
fn expr_name_chain(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<Vec<Symbol>> {
    match &expr.kind {
        ExprKind::Ident(reses) => {
            let res = reses.first()?;
            let name = match *res {
                Res::Item(ItemId::Variable(vid)) => gcx.hir.variable(vid).name?.name,
                Res::Item(ItemId::Function(fid)) => gcx.hir.function(fid).name?.name,
                Res::Item(ItemId::Contract(cid)) => gcx.hir.contract(cid).name.name,
                Res::Builtin(b) => b.name(),
                _ => return None,
            };
            Some(vec![name])
        }
        ExprKind::Member(lhs, ident) => {
            let mut chain = expr_name_chain(gcx, lhs)?;
            chain.insert(0, ident.name);
            Some(chain)
        }
        _ => None,
    }
}

/// Infers a custom type's true type by recursing through the HIR.
///
/// `custom_type` is a name chain ordered outermost-first (root at back). This is mutated during
/// resolution. `contract_id` narrows the search to a specific contract scope.
fn infer_custom_type(
    gcx: Gcx<'_>,
    custom_type: &mut Vec<Symbol>,
    contract_id: Option<ContractId>,
) -> Result<Option<DynSolType>> {
    if let Some(last) = custom_type.last()
        && (last.as_str() == "this" || last.as_str() == "super")
    {
        custom_type.pop();
    }
    if custom_type.is_empty() {
        return Ok(None);
    }

    if let Some(cid) = contract_id {
        let hir = &gcx.hir;
        let contract = hir.contract(cid);

        let cur_name = *custom_type.last().unwrap();
        let cur = cur_name.as_str();

        // Function?
        if let Some(fid) = contract
            .functions()
            .find(|&f| hir.function(f).name.as_ref().map(|n| n.as_str() == cur).unwrap_or(false))
        {
            let func = hir.function(fid);
            if let res @ Some(_) = func_members(func, custom_type) {
                return Ok(res);
            }

            if func.returns.is_empty() {
                eyre::bail!(
                    "This call expression does not return any values to inspect. Insert as statement."
                )
            }

            let sm = func.state_mutability;
            if !matches!(sm, StateMutability::View | StateMutability::Pure) {
                eyre::bail!("This function mutates state. Insert as a statement.")
            }

            let ret_id = func.returns[0];
            let ret_var = hir.variable(ret_id);
            return Ok(solar_ty_to_dyn(gcx, gcx.type_of_item(ret_id.into()))
                .or_else(|| hir_ty_to_dyn(gcx, &ret_var.ty)));
        }

        // Variable?
        if let Some(vid) = contract
            .variables()
            .find(|&v| hir.variable(v).name.as_ref().map(|n| n.as_str() == cur).unwrap_or(false))
        {
            if let Some(ty) = solar_ty_to_dyn(gcx, gcx.type_of_item(vid.into())) {
                custom_type.pop();
                if custom_type.is_empty() {
                    return Ok(Some(ty));
                }
                let next_member = custom_type.drain(..).next().unwrap_or(Symbol::DUMMY);
                return Ok(dyn_member(&ty, next_member.as_str()).or(Some(ty)));
            }
            let var = hir.variable(vid);
            return infer_var_ty(gcx, &var.ty, custom_type);
        }

        // Struct?
        if let Some(sid) = contract.items.iter().find_map(|i| {
            if let ItemId::Struct(sid) = i
                && hir.strukt(*sid).name.as_str() == cur
            {
                Some(*sid)
            } else {
                None
            }
        }) {
            let inner = gcx
                .struct_field_types(sid)
                .iter()
                .map(|&t| {
                    solar_ty_to_dyn(gcx, t)
                        .ok_or_else(|| eyre::eyre!("Struct `{cur}` has invalid fields"))
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(Some(DynSolType::Tuple(inner)));
        }

        eyre::bail!(
            "Could not find any definition in contract \"{}\" for type: {custom_type:?}",
            contract.name.as_str()
        )
    }

    let repl_id = gcx
        .hir
        .contracts_enumerated()
        .find_map(|(cid, c)| (c.name.as_str() == "REPL").then_some(cid));
    if let Some(repl_id) = repl_id
        && let Ok(res) = infer_custom_type(gcx, custom_type, Some(repl_id))
    {
        return Ok(res);
    }

    let last_name = *custom_type.last().unwrap();
    let last = last_name.as_str();
    let contract_match = gcx
        .hir
        .contracts_enumerated()
        .find_map(|(cid, c)| (c.name.as_str() == last).then_some(cid));
    if let Some(cid) = contract_match {
        custom_type.pop();
        return infer_custom_type(gcx, custom_type, Some(cid));
    }

    Ok(None)
}

/// Infers the type from a variable's HIR type, optionally accessing a named member.
fn infer_var_ty(
    gcx: Gcx<'_>,
    ty: &HirType<'_>,
    custom_type: &mut Vec<Symbol>,
) -> Result<Option<DynSolType>> {
    let Some(ty) = hir_ty_to_dyn(gcx, ty) else { return Ok(None) };
    let next_member = custom_type.drain(..).next();
    if let Some(m) = next_member {
        Ok(dyn_member(&ty, m.as_str()).or(Some(ty)))
    } else {
        Ok(Some(ty))
    }
}

/// Get the return type of a contract method call `receiver.method()`.
fn get_function_return_type(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<DynSolType> {
    let ExprKind::Call(callee, _, _) = &expr.kind else { return None };
    let ExprKind::Member(obj, fn_ident) = &callee.kind else { return None };
    let ExprKind::Ident(reses) = &obj.kind else { return None };
    let res = reses.first()?;
    let var_id = match res {
        Res::Item(ItemId::Variable(vid)) => *vid,
        _ => return None,
    };
    let var_ty = gcx.type_of_item(var_id.into()).peel_refs();
    let cid = match var_ty.kind {
        TyKind::Contract(cid) => cid,
        _ => return None,
    };

    let hir = &gcx.hir;
    let contract = hir.contract(cid);
    let fid = contract
        .functions()
        .find(|&f| hir.function(f).name.as_ref().map(|n| n.as_str()) == Some(fn_ident.as_str()))?;
    let func = hir.function(fid);
    let ret_id = *func.returns.first()?;
    solar_ty_to_dyn(gcx, gcx.type_of_item(ret_id.into()))
}

/// Returns Some if the custom type is a function member access.
///
/// Ref: <https://docs.soliditylang.org/en/latest/types.html#function-types>
#[inline]
fn func_members(func: &Function<'_>, custom_type: &[Symbol]) -> Option<DynSolType> {
    if !matches!(func.kind, FunctionKind::Function) {
        return None;
    }
    if !matches!(func.visibility, Visibility::External | Visibility::Public) {
        return None;
    }
    match custom_type.first().unwrap().as_str() {
        "address" => Some(DynSolType::Address),
        "selector" => Some(DynSolType::FixedBytes(4)),
        _ => None,
    }
}

/// Whether execution should continue after inspecting this expression.
#[inline]
fn should_continue(expr: &Expr<'_>) -> bool {
    match &expr.kind {
        // assignments and compound assignments
        ExprKind::Assign(_, _, _) => true,
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

/// Parses an [`Expr`] number/hex literal into a `U256`. Returns `None` if the expression
/// is not a numeric literal.
///
/// SubDenominations are already applied to numeric literals in solar's HIR.
const fn parse_number_literal(expr: &Expr<'_>) -> Option<U256> {
    match &expr.kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Number(n) => Some(*n),
            _ => None,
        },
        _ => None,
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
fn solar_ty_to_dyn<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> Option<DynSolType> {
    match ty.kind {
        TyKind::Elementary(et) => elementary_to_dyn(et),
        TyKind::Ref(inner, _) => solar_ty_to_dyn(gcx, inner),
        TyKind::Array(elem, n) => {
            let inner = solar_ty_to_dyn(gcx, elem)?;
            let size: usize = n.try_into().ok()?;
            Some(DynSolType::FixedArray(Box::new(inner), size))
        }
        TyKind::DynArray(elem) | TyKind::Slice(elem) => {
            let inner = solar_ty_to_dyn(gcx, elem)?;
            Some(DynSolType::Array(Box::new(inner)))
        }
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
        TyKind::FnPtr(f) => match f.returns.len() {
            0 => None,
            1 => solar_ty_to_dyn(gcx, f.returns[0]),
            _ => Some(DynSolType::Tuple(
                f.returns.iter().filter_map(|t| solar_ty_to_dyn(gcx, *t)).collect(),
            )),
        },
        TyKind::Type(inner) => solar_ty_to_dyn(gcx, inner),
        TyKind::Meta(inner) => solar_ty_to_dyn(gcx, inner),
        TyKind::IntLiteral(neg, size) => {
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
    use solar::sema::Compiler;
    use std::sync::Mutex;

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
                ("abi.encodeCall(func(), (_, _))", Bytes),
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
    /// Tests bypass `SessionSource::build` (which routes through foundry-compilers + solc) so that
    /// inputs which are syntactically valid but semantically rejected by solc (e.g.
    /// `abi.decode(bytes, (uint8[13]))` or `a[0:3]` on a memory array) can still exercise the
    /// HIR-based type-inference engine.
    fn get_type_ethabi(s: &mut SessionSource, input: &str, clear: bool) -> Option<DynSolType> {
        if clear {
            s.clear();
        }

        // Always declare a sample enum so `Enum1` is available for `type(Enum1)` tests.
        *s = s.clone_with_new_line("enum Enum1 { A }".into()).unwrap().0;

        let input = format!("{};", input.trim_end().trim_end_matches(';'));
        let (new_source, _) = s.clone_with_new_line(input).unwrap();
        *s = new_source.clone();

        let src = new_source.to_repl_source();
        let sess =
            solar::interface::Session::builder().with_buffer_emitter(Default::default()).build();
        let mut compiler = Compiler::new(sess);

        compiler.enter_mut(|c| -> Option<DynSolType> {
            // Stage 1: parse + lower (mutable access required).
            let lowered = {
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
            };
            if !lowered {
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
            expr_to_dyn(gcx, expr, true)
        })
    }

    fn generic_type_test<'a, T, I>(s: &mut SessionSource, input: I)
    where
        T: AsRef<str> + std::fmt::Display + 'a,
        I: IntoIterator<Item = &'a (T, DynSolType)> + 'a,
    {
        for (input, expected) in input {
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
