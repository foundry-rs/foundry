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
        hir::{self, Visibility},
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
            let hir::StmtKind::DeclSingle(vid) = last.kind else { return None };
            let var = gcx.hir.variable(vid);
            let init = var.initializer?;
            let hir::ExprKind::Call(_callee, args, _) = &init.kind else { return None };
            let inner_expr = args.exprs().next()?;

            // If the call is `func()` returning a single value, prefer the function return type.
            if let Some(ty) = Type::get_function_return_type(gcx, inner_expr) {
                return Some((should_continue(inner_expr), ty));
            }

            let ty = Type::ethabi(gcx, inner_expr, true)?;
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
            hir::StmtKind::DeclSingle(vid) => {
                let var = hir.variable(vid);
                if var.name.map(|n| n.as_str() == name).unwrap_or(false) {
                    return solar_ty_to_dyn(gcx, gcx.type_of_item(vid.into()));
                }
            }
            hir::StmtKind::DeclMulti(vids, _) => {
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

/// Formats an [`hir::Event`] into an inspection message.
// TODO: Verbosity option
fn format_event_definition(gcx: Gcx<'_>, event: &hir::Event<'_>) -> Result<String> {
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
    Array(Box<Self>),

    /// (type, length)
    FixedArray(Box<Self>, usize),

    /// (type, index)
    ArrayIndex(Box<Self>, Option<usize>),

    /// (types)
    Tuple(Vec<Option<Self>>),

    /// (name, params, returns)
    Function(Box<Self>, Vec<Option<Self>>, Vec<Option<Self>>),

    /// (lhs, rhs)
    Access(Box<Self>, Symbol),

    /// (types)
    Custom(Vec<Symbol>),
}

impl Type {
    /// Convert a [`hir::Expr`] to a [`Type`].
    fn from_expr(gcx: Gcx<'_>, expr: &hir::Expr<'_>) -> Option<Self> {
        match &expr.kind {
            // Elementary type expression: `uint256`, `address`, etc.
            hir::ExprKind::Type(ty) => Self::from_hir_ty(gcx, ty),

            // `type(T)` expression. Modelled like a call to a builtin `type` function so that
            // member access (e.g. `type(C).name` or `type(uint256).max`) can be resolved by
            // [`Self::map_special`].
            hir::ExprKind::TypeCall(ty) => {
                let inner = Self::from_hir_ty(gcx, ty);
                Some(Self::Function(
                    Box::new(Self::Custom(vec![Symbol::intern("type")])),
                    vec![inner],
                    vec![],
                ))
            }

            // Resolved identifier: `foo`.
            hir::ExprKind::Ident(reses) => {
                let res = reses.first()?;
                match *res {
                    hir::Res::Item(item) => match item {
                        hir::ItemId::Variable(vid) => {
                            let var = gcx.hir.variable(vid);
                            let name = var.name?.name;
                            // Try the resolved solar type first.
                            let ty = gcx.type_of_item(vid.into());
                            if let Some(dyn_ty) = solar_ty_to_dyn(gcx, ty) {
                                Some(Self::Builtin(dyn_ty))
                            } else {
                                Some(Self::Custom(vec![name]))
                            }
                        }
                        hir::ItemId::Function(fid) => {
                            let func = gcx.hir.function(fid);
                            let name = func.name?.name;
                            Some(Self::Custom(vec![name]))
                        }
                        hir::ItemId::Contract(cid) => {
                            let c = gcx.hir.contract(cid);
                            Some(Self::Custom(vec![c.name.name]))
                        }
                        hir::ItemId::Struct(sid) => {
                            // Struct constructor: produces a value of the struct type, which we
                            // model as a tuple of field types.
                            let fields = gcx.struct_field_types(sid);
                            let parts = fields
                                .iter()
                                .map(|&t| solar_ty_to_dyn(gcx, t).map(Self::Builtin))
                                .collect();
                            Some(Self::Tuple(parts))
                        }
                        hir::ItemId::Enum(eid) => {
                            let e = gcx.hir.enumm(eid);
                            Some(Self::Custom(vec![e.name.name]))
                        }
                        hir::ItemId::Udvt(uid) => {
                            let u = gcx.hir.udvt(uid);
                            Some(Self::Custom(vec![u.name.name]))
                        }
                        hir::ItemId::Error(eid) => {
                            let e = gcx.hir.error(eid);
                            Some(Self::Custom(vec![e.name.name]))
                        }
                        hir::ItemId::Event(eid) => {
                            let e = gcx.hir.event(eid);
                            Some(Self::Custom(vec![e.name.name]))
                        }
                    },
                    hir::Res::Builtin(b) => Some(Self::Custom(vec![b.name()])),
                    hir::Res::Namespace(_) | hir::Res::Err(_) => None,
                }
            }

            // Index/access: `arr[i]`, `MyType[]`.
            hir::ExprKind::Index(base, idx) => Self::from_expr(gcx, base).map(|ty| {
                let boxed = Box::new(ty);
                let num =
                    idx.and_then(|e| parse_number_literal(e)).and_then(|n| usize::try_from(n).ok());
                match &base.kind {
                    hir::ExprKind::Type(_) | hir::ExprKind::TypeCall(_) => {
                        if let Some(num) = num {
                            Self::FixedArray(boxed, num)
                        } else {
                            Self::Array(boxed)
                        }
                    }
                    _ => Self::ArrayIndex(boxed, num),
                }
            }),

            // Slice expression: `arr[a:b]`.
            hir::ExprKind::Slice(base, _, _) => Self::from_expr(gcx, base),

            // Array literal `[a, b, c]`.
            hir::ExprKind::Array(values) => values
                .first()
                .and_then(|e| Self::from_expr(gcx, e))
                .map(|ty| Self::FixedArray(Box::new(ty), values.len())),

            // Tuple expression `(a, b, c)`.
            hir::ExprKind::Tuple(items) => Some(Self::Tuple(
                items.iter().map(|opt| opt.and_then(|e| Self::from_expr(gcx, e))).collect(),
            )),

            // Member access `lhs.rhs`.
            hir::ExprKind::Member(lhs, ident) => {
                Self::from_expr(gcx, lhs).map(|lhs| Self::Access(Box::new(lhs), ident.name))
            }

            // `new T`.
            hir::ExprKind::New(ty) => Self::from_hir_ty(gcx, ty),

            // `payable(addr)`.
            hir::ExprKind::Payable(_) => Some(Self::Builtin(DynSolType::Address)),

            // Ternary: prefer truthy branch's type, fall back to else branch.
            hir::ExprKind::Ternary(_, t, e) => {
                Self::from_expr(gcx, t).or_else(|| Self::from_expr(gcx, e))
            }

            // Delete expression has no usable type.
            hir::ExprKind::Delete(_) => None,

            // Literals.
            hir::ExprKind::Lit(lit) => match &lit.kind {
                LitKind::Address(_) => Some(Self::Builtin(DynSolType::Address)),
                LitKind::Bool(_) => Some(Self::Builtin(DynSolType::Bool)),
                LitKind::Str(kind, _, _) => match kind {
                    StrKind::Hex => Some(Self::Builtin(DynSolType::Bytes)),
                    StrKind::Str | StrKind::Unicode => Some(Self::Builtin(DynSolType::String)),
                },
                LitKind::Number(_) | LitKind::Rational(_) => {
                    Some(Self::Builtin(DynSolType::Uint(256)))
                }
                LitKind::Err(_) => None,
            },

            // Unary operations.
            hir::ExprKind::Unary(op, inner) => match op.kind {
                UnOpKind::Neg => Self::from_expr(gcx, inner).map(Self::invert_int),
                UnOpKind::Not => Some(Self::Builtin(DynSolType::Bool)),
                UnOpKind::BitNot
                | UnOpKind::PreInc
                | UnOpKind::PreDec
                | UnOpKind::PostInc
                | UnOpKind::PostDec => Self::from_expr(gcx, inner),
            },

            // Binary operations.
            hir::ExprKind::Binary(lhs, op, rhs) => match op.kind {
                BinOpKind::Lt
                | BinOpKind::Le
                | BinOpKind::Gt
                | BinOpKind::Ge
                | BinOpKind::Eq
                | BinOpKind::Ne
                | BinOpKind::And
                | BinOpKind::Or => Some(Self::Builtin(DynSolType::Bool)),
                BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul | BinOpKind::Div => {
                    match (Self::ethabi(gcx, lhs, false), Self::ethabi(gcx, rhs, false)) {
                        (
                            Some(DynSolType::Int(_) | DynSolType::Uint(_)),
                            Some(DynSolType::Int(_)),
                        )
                        | (Some(DynSolType::Int(_)), Some(DynSolType::Uint(_))) => {
                            Some(Self::Builtin(DynSolType::Int(256)))
                        }
                        _ => Some(Self::Builtin(DynSolType::Uint(256))),
                    }
                }
                BinOpKind::Rem
                | BinOpKind::Pow
                | BinOpKind::BitAnd
                | BinOpKind::BitOr
                | BinOpKind::BitXor
                | BinOpKind::Shl
                | BinOpKind::Shr
                | BinOpKind::Sar => Some(Self::Builtin(DynSolType::Uint(256))),
            },

            // Assignments: type of the lhs.
            hir::ExprKind::Assign(lhs, _, _) => Self::from_expr(gcx, lhs),

            // Function call.
            hir::ExprKind::Call(callee, args, _named) => Self::from_expr(gcx, callee).map(|name| {
                let args = args.exprs().map(|e| Self::from_expr(gcx, e)).collect();
                Self::Function(Box::new(name), args, vec![])
            }),

            hir::ExprKind::Err(_) => None,
        }
    }

    /// Convert a [`hir::Type`] to a [`Type`].
    fn from_hir_ty(gcx: Gcx<'_>, ty: &hir::Type<'_>) -> Option<Self> {
        match &ty.kind {
            hir::TypeKind::Elementary(et) => Some(Self::Builtin(elementary_to_dyn(*et)?)),
            hir::TypeKind::Array(arr) => {
                let elem = Self::from_hir_ty(gcx, &arr.element)?;
                if let Some(size) = arr.size {
                    let n = parse_number_literal(size).and_then(|n| usize::try_from(n).ok());
                    if let Some(n) = n {
                        Some(Self::FixedArray(Box::new(elem), n))
                    } else {
                        Some(Self::Array(Box::new(elem)))
                    }
                } else {
                    Some(Self::Array(Box::new(elem)))
                }
            }
            hir::TypeKind::Function(f) => {
                let params = f
                    .parameters
                    .iter()
                    .map(|&pid| {
                        let var = gcx.hir.variable(pid);
                        Self::from_hir_ty(gcx, &var.ty)
                    })
                    .collect();
                let returns = f
                    .returns
                    .iter()
                    .map(|&pid| {
                        let var = gcx.hir.variable(pid);
                        Self::from_hir_ty(gcx, &var.ty)
                    })
                    .collect();
                Some(Self::Function(
                    Box::new(Self::Custom(vec![Symbol::intern("__fn_type__")])),
                    params,
                    returns,
                ))
            }
            hir::TypeKind::Mapping(m) => Self::from_hir_ty(gcx, &m.value),
            hir::TypeKind::Custom(item) => {
                // User-defined type names always become `Custom([name])` here, mirroring the
                // legacy pt-based engine where custom names came in via `Variable(name)` rather
                // than the elementary `from_type` path. Conversion to a concrete `DynSolType`
                // (e.g. struct → tuple, udvt → underlying) happens later in `try_as_ethabi`.
                let name = match *item {
                    hir::ItemId::Contract(id) => gcx.hir.contract(id).name.name,
                    hir::ItemId::Struct(id) => gcx.hir.strukt(id).name.name,
                    hir::ItemId::Enum(id) => gcx.hir.enumm(id).name.name,
                    hir::ItemId::Udvt(id) => gcx.hir.udvt(id).name.name,
                    hir::ItemId::Error(id) => gcx.hir.error(id).name.name,
                    hir::ItemId::Event(id) => gcx.hir.event(id).name.name,
                    hir::ItemId::Function(id) => gcx.hir.function(id).name?.name,
                    hir::ItemId::Variable(id) => gcx.hir.variable(id).name?.name,
                };
                Some(Self::Custom(vec![name]))
            }
            hir::TypeKind::Err(_) => {
                // Best-effort fallback: when name resolution failed, recover the textual name
                // from the source span so the inference engine can still treat it as a custom
                // type (e.g., `type(C).name` where contract `C` does not exist).
                let snippet = gcx.sess.source_map().span_to_snippet(ty.span).ok()?;
                let name = snippet.trim();
                if name.is_empty() { None } else { Some(Self::Custom(vec![Symbol::intern(name)])) }
            }
        }
    }

    /// Handle special expressions like
    /// [global variables](https://docs.soliditylang.org/en/latest/cheatsheet.html#global-variables).
    fn map_special(self) -> Self {
        if !matches!(self, Self::Function(_, _, _) | Self::Access(_, _) | Self::Custom(_)) {
            return self;
        }

        let mut types: Vec<Symbol> = Vec::with_capacity(5);
        let mut args = None;
        self.recurse(&mut types, &mut args);

        let len = types.len();
        if len == 0 {
            return self;
        }

        // Type members, like array, bytes etc
        #[expect(clippy::single_match)]
        #[allow(clippy::collapsible_match)]
        match &self {
            Self::Access(inner, access) => {
                if let Some(ty) = inner.as_ref().clone().try_as_ethabi(false, None) {
                    // Array / bytes members
                    let ty = Self::Builtin(ty);
                    match access.as_str() {
                        "length" if ty.is_dynamic() || ty.is_array() || ty.is_fixed_bytes() => {
                            return Self::Builtin(DynSolType::Uint(256));
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
                            "timestamp" | "difficulty" | "prevrandao" | "number" | "gaslimit"
                            | "chainid" | "basefee" | "blobbasefee" => Some(DynSolType::Uint(256)),
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
                                let mut args = args.unwrap();
                                let last = args.pop().unwrap();
                                match last {
                                    Some(ty) => {
                                        return match ty {
                                            Self::Tuple(_) => ty,
                                            ty => Self::Tuple(vec![Some(ty)]),
                                        };
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
    /// are found.
    fn recurse(&self, types: &mut Vec<Symbol>, args: &mut Option<Vec<Option<Self>>>) {
        match self {
            Self::Builtin(ty) => types.push(Symbol::intern(&ty.to_string())),
            Self::Custom(tys) => types.extend(tys.iter().copied()),
            Self::Access(expr, name) => {
                types.push(*name);
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

    /// Infers a custom type's true type by recursing through the HIR.
    fn infer_custom_type(
        gcx: Gcx<'_>,
        custom_type: &mut Vec<Symbol>,
        contract_id: Option<hir::ContractId>,
    ) -> Result<Option<DynSolType>> {
        if let Some(last) = custom_type.last()
            && (last.as_str() == "this" || last.as_str() == "super")
        {
            custom_type.pop();
        }
        if custom_type.is_empty() {
            return Ok(None);
        }

        // If a contract exists with the given name, check its definitions for a match.
        // Otherwise look in the `run`.
        if let Some(cid) = contract_id {
            let hir = &gcx.hir;
            let contract = hir.contract(cid);

            let cur_name = *custom_type.last().unwrap();
            let cur = cur_name.as_str();

            // Function?
            if let Some(fid) = contract.functions().find(|&f| {
                hir.function(f).name.as_ref().map(|n| n.as_str() == cur).unwrap_or(false)
            }) {
                let func = hir.function(fid);
                if let res @ Some(_) = func_members(func, custom_type) {
                    return Ok(res);
                }

                if func.returns.is_empty() {
                    eyre::bail!(
                        "This call expression does not return any values to inspect. Insert as statement."
                    )
                }

                // Check if our final function call alters the state. If it does, we bail so that it
                // will be inserted normally without inspecting.
                let sm = func.state_mutability;
                if !matches!(sm, StateMutability::View | StateMutability::Pure) {
                    eyre::bail!("This function mutates state. Insert as a statement.")
                }

                // Resolve the return type.
                let ret_id = func.returns[0];
                let ret_var = hir.variable(ret_id);
                // If the return type is a custom (user-defined) type, recurse on the same contract.
                if let hir::TypeKind::Custom(_) = &ret_var.ty.kind
                    && let Some(t) = Self::from_hir_ty(gcx, &ret_var.ty)
                {
                    return Ok(t.try_as_ethabi(true, Some(gcx)));
                }
                return Ok(solar_ty_to_dyn(gcx, gcx.type_of_item(ret_id.into())));
            }

            // Variable?
            if let Some(vid) = contract.variables().find(|&v| {
                hir.variable(v).name.as_ref().map(|n| n.as_str() == cur).unwrap_or(false)
            }) {
                let var = hir.variable(vid);
                if let Some(ty) = solar_ty_to_dyn(gcx, gcx.type_of_item(vid.into())) {
                    // Check if there are remaining members to access.
                    custom_type.pop();
                    if custom_type.is_empty() {
                        return Ok(Some(ty));
                    }
                    // Try to use the resolved type for member-access lookups.
                    let access = Self::Access(
                        Box::new(Self::Builtin(ty.clone())),
                        custom_type.drain(..).next().unwrap_or(Symbol::DUMMY),
                    );
                    if let Some(mapped) = access.map_special().try_as_ethabi(true, Some(gcx)) {
                        return Ok(Some(mapped));
                    }
                    return Ok(Some(ty));
                }
                // Fall back to the type expression.
                return Self::infer_var_ty(gcx, &var.ty, custom_type);
            }

            // Struct?
            if let Some(sid) = contract.items.iter().find_map(|i| {
                if let hir::ItemId::Struct(sid) = i
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
        // Check if the custom type is a variable or function within the REPL contract first.
        let repl_id = gcx
            .hir
            .contracts_enumerated()
            .find_map(|(cid, c)| (c.name.as_str() == "REPL").then_some(cid));
        if let Some(repl_id) = repl_id
            && let Ok(res) = Self::infer_custom_type(gcx, custom_type, Some(repl_id))
        {
            return Ok(res);
        }

        // Check if the first element of the custom type is a known contract.
        let last_name = *custom_type.last().unwrap();
        let last = last_name.as_str();
        let contract_match = gcx
            .hir
            .contracts_enumerated()
            .find_map(|(cid, c)| (c.name.as_str() == last).then_some(cid));
        if let Some(cid) = contract_match {
            custom_type.pop();
            return Self::infer_custom_type(gcx, custom_type, Some(cid));
        }

        // Otherwise gracefully give up.
        Ok(None)
    }

    /// Infers the type from a variable's HIR type.
    fn infer_var_ty(
        gcx: Gcx<'_>,
        ty: &hir::Type<'_>,
        custom_type: &mut Vec<Symbol>,
    ) -> Result<Option<DynSolType>> {
        let res: Option<DynSolType> = if let Some(t) = Self::from_hir_ty(gcx, ty) {
            t.try_as_ethabi(true, Some(gcx))
        } else {
            None
        };
        match res {
            Some(ty) => {
                let access = Self::Access(
                    Box::new(Self::Builtin(ty.clone())),
                    custom_type.drain(..).next().unwrap_or(Symbol::DUMMY),
                );
                if let Some(mapped) = access.map_special().try_as_ethabi(true, Some(gcx)) {
                    Ok(Some(mapped))
                } else {
                    Ok(Some(ty))
                }
            }
            None => Ok(None),
        }
    }

    /// Attempt to convert this type into a [`DynSolType`].
    ///
    /// `lookup` controls whether [`Self::Custom`] entries should be resolved via the HIR.
    fn try_as_ethabi(self, lookup: bool, gcx: Option<Gcx<'_>>) -> Option<DynSolType> {
        match self {
            Self::Builtin(ty) => Some(ty),
            Self::Tuple(types) => Some(DynSolType::Tuple(types_to_parameters(types, lookup, gcx))),
            Self::Array(inner) => match *inner {
                ty @ Self::Custom(_) => ty.try_as_ethabi(lookup, gcx),
                _ => {
                    inner.try_as_ethabi(lookup, gcx).map(|inner| DynSolType::Array(Box::new(inner)))
                }
            },
            Self::FixedArray(inner, size) => match *inner {
                ty @ Self::Custom(_) => ty.try_as_ethabi(lookup, gcx),
                _ => inner
                    .try_as_ethabi(lookup, gcx)
                    .map(|inner| DynSolType::FixedArray(Box::new(inner), size)),
            },
            ty @ Self::ArrayIndex(_, _) => ty.into_array_index(lookup, gcx),
            Self::Function(ty, _, _) => ty.try_as_ethabi(lookup, gcx),
            // should have been mapped to `Custom` in previous steps
            Self::Access(_, _) => None,
            Self::Custom(mut types) => {
                if !lookup {
                    return None;
                }
                gcx.and_then(|gcx| Self::infer_custom_type(gcx, &mut types, None).ok().flatten())
            }
        }
    }

    /// Equivalent to `Type::from_expr` + `Type::map_special` + `Type::try_as_ethabi`.
    fn ethabi(gcx: Gcx<'_>, expr: &hir::Expr<'_>, lookup: bool) -> Option<DynSolType> {
        Self::from_expr(gcx, expr)
            .map(Self::map_special)
            .and_then(|ty| ty.try_as_ethabi(lookup, Some(gcx)))
    }

    /// Get the return type of a function call expression.
    fn get_function_return_type<'a>(gcx: Gcx<'_>, expr: &'a hir::Expr<'a>) -> Option<DynSolType> {
        let hir::ExprKind::Call(callee, _, _) = &expr.kind else { return None };
        let hir::ExprKind::Member(obj, fn_ident) = &callee.kind else { return None };
        // The receiver should be a variable holding a contract.
        let hir::ExprKind::Ident(reses) = &obj.kind else { return None };
        let res = reses.first()?;
        let var_id = match res {
            hir::Res::Item(hir::ItemId::Variable(vid)) => *vid,
            _ => return None,
        };
        let var_ty = gcx.type_of_item(var_id.into()).peel_refs();
        let cid = match var_ty.kind {
            TyKind::Contract(cid) => cid,
            _ => return None,
        };

        let hir = &gcx.hir;
        let contract = hir.contract(cid);
        let fid = contract.functions().find(|&f| {
            hir.function(f).name.as_ref().map(|n| n.as_str()) == Some(fn_ident.as_str())
        })?;
        let func = hir.function(fid);
        let ret_id = *func.returns.first()?;
        solar_ty_to_dyn(gcx, gcx.type_of_item(ret_id.into()))
    }

    /// Inverts Int to Uint and vice-versa.
    fn invert_int(self) -> Self {
        match self {
            Self::Builtin(DynSolType::Uint(n)) => Self::Builtin(DynSolType::Int(n)),
            Self::Builtin(DynSolType::Int(n)) => Self::Builtin(DynSolType::Uint(n)),
            x => x,
        }
    }

    /// Returns the `DynSolType` contained by `Type::Builtin`.
    #[inline]
    fn into_builtin(self) -> Option<DynSolType> {
        match self {
            Self::Builtin(ty) => Some(ty),
            _ => None,
        }
    }

    /// Returns the resulting `DynSolType` of indexing self.
    fn into_array_index(self, lookup: bool, gcx: Option<Gcx<'_>>) -> Option<DynSolType> {
        match self {
            Self::Array(inner) | Self::FixedArray(inner, _) | Self::ArrayIndex(inner, _) => {
                match inner.try_as_ethabi(lookup, gcx) {
                    Some(DynSolType::Array(inner) | DynSolType::FixedArray(inner, _)) => {
                        Some(*inner)
                    }
                    Some(DynSolType::Bytes | DynSolType::String | DynSolType::FixedBytes(_)) => {
                        Some(DynSolType::FixedBytes(1))
                    }
                    ty => ty,
                }
            }
            _ => None,
        }
    }

    /// Returns whether this type is dynamic.
    #[inline]
    const fn is_dynamic(&self) -> bool {
        match self {
            // TODO: Note, this is not entirely correct. Fixed arrays of non-dynamic types are
            // not dynamic, nor are tuples of non-dynamic types.
            Self::Builtin(DynSolType::Bytes | DynSolType::String | DynSolType::Array(_)) => true,
            Self::Array(_) => true,
            _ => false,
        }
    }

    /// Returns whether this type is an array.
    #[inline]
    const fn is_array(&self) -> bool {
        matches!(
            self,
            Self::Array(_)
                | Self::FixedArray(_, _)
                | Self::Builtin(DynSolType::Array(_) | DynSolType::FixedArray(_, _))
        )
    }

    /// Returns whether this type is a dynamic array (can call push, pop).
    #[inline]
    const fn is_dynamic_array(&self) -> bool {
        matches!(self, Self::Array(_) | Self::Builtin(DynSolType::Array(_)))
    }

    const fn is_fixed_bytes(&self) -> bool {
        matches!(self, Self::Builtin(DynSolType::FixedBytes(_)))
    }
}

/// Returns Some if the custom type is a function member access.
///
/// Ref: <https://docs.soliditylang.org/en/latest/types.html#function-types>
#[inline]
fn func_members(func: &hir::Function<'_>, custom_type: &[Symbol]) -> Option<DynSolType> {
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
fn should_continue(expr: &hir::Expr<'_>) -> bool {
    match &expr.kind {
        // assignments and compound assignments
        hir::ExprKind::Assign(_, _, _) => true,
        // ++/-- pre/post operations
        hir::ExprKind::Unary(op, _) => matches!(
            op.kind,
            UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
        ),
        // Array.pop()
        hir::ExprKind::Call(callee, _, _) => match &callee.kind {
            hir::ExprKind::Member(_, ident) => ident.as_str() == "pop",
            _ => false,
        },
        _ => false,
    }
}

fn types_to_parameters(
    types: Vec<Option<Type>>,
    lookup: bool,
    gcx: Option<Gcx<'_>>,
) -> Vec<DynSolType> {
    types.into_iter().filter_map(|ty| ty.and_then(|ty| ty.try_as_ethabi(lookup, gcx))).collect()
}

/// Parses an [`hir::Expr`] number/hex literal into a `U256`. Returns `None` if the expression
/// is not a numeric literal.
///
/// SubDenominations are already applied to numeric literals in solar's HIR.
const fn parse_number_literal(expr: &hir::Expr<'_>) -> Option<U256> {
    match &expr.kind {
        hir::ExprKind::Lit(lit) => match &lit.kind {
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
        use solar::sema::Compiler;

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
                hir::StmtKind::Expr(e) => e,
                _ => return None,
            };
            Type::ethabi(gcx, expr, true)
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
