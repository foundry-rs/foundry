//! Executor
//!
//! This module contains the execution logic for the [SessionSource].

use crate::prelude::{
    ChiselDispatcher, ChiselResult, ChiselRunner, IntermediateOutput, SessionSource,
};
use core::fmt::Debug;
use ethers::{
    abi::{ethabi, ParamType, Token},
    types::{Address, I256, U256},
    utils::hex,
};
use ethers_solc::Artifact;
use eyre::{Result, WrapErr};
use forge::{
    decode::decode_console_logs,
    executor::{inspector::CheatsConfig, Backend, ExecutorBuilder},
};
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
                runner.run(bytecode.into_owned())
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
    pub async fn inspect(&mut self, input: &str) -> Result<Option<String>> {
        let mut source = if let Ok((source, _)) =
            self.clone_with_new_line(format!("bytes memory inspectoor = abi.encode({input})"))
        {
            source
        } else {
            return Ok(None)
        };

        let mut res = if let Ok((_, res)) = source.execute().await { res } else { return Ok(None) };

        if let Some((stack, memory, _)) = &res.state {
            let generated_output = source
                .generated_output
                .as_ref()
                .ok_or(eyre::eyre!("Could not find generated output!"))?;

            // If the expression is a variable declaration within the REPL contract, use its type;
            // otherwise, attempt to infer the type.
            let contract_expr = generated_output
                .intermediate
                .repl_contract_expressions
                .get(input)
                .or_else(|| self.infer_inner_expr_type(&source));
            eprintln!("{contract_expr:?}");

            let ty =
                match contract_expr.and_then(|e| Type::ethabi(e, &generated_output.intermediate)) {
                    Some(ty) => ty,
                    // this type was denied for inspection, thus we move on gracefully
                    None => return Ok(None),
                };
            eprintln!("{ty:?}");

            let memory_offset = if let Some(offset) = stack.data().last() {
                offset.as_usize()
            } else {
                eyre::bail!("No result found");
            };
            if memory_offset + 32 > memory.len() {
                eyre::bail!("Memory size insufficient");
            }
            let data = &memory.data()[memory_offset + 32..];
            // TODO: Encode array length and relative offset for dynamic arrays
            let mut tokens = ethabi::decode(&[ty], data).wrap_err("Could not decode ABI")?;

            tokens.pop().map_or(Err(eyre::eyre!("No tokens decoded")), |token| {
                Ok(Some(format_token(token)))
            })
        } else {
            if let Ok(decoder) = ChiselDispatcher::decode_traces(&source.config, &mut res) {
                if ChiselDispatcher::show_traces(&decoder, &mut res).await.is_err() {
                    eyre::bail!("Failed to display traces");
                };

                // Show console logs, if there are any
                let decoded_logs = decode_console_logs(&res.logs);
                if !decoded_logs.is_empty() {
                    println!("{}", Paint::green("Logs:"));
                    for log in decoded_logs {
                        println!("  {log}");
                    }
                }
            }
            eyre::bail!("Failed to inspect expression")
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
    fn infer_inner_expr_type<'s>(
        &mut self,
        source: &'s SessionSource,
    ) -> Option<&'s pt::Expression> {
        let out = source.generated_output.as_ref()?;
        let run = out.intermediate.run_func_body().ok()?.last();
        match run {
            Some(pt::Statement::VariableDefinition(
                _,
                _,
                Some(pt::Expression::FunctionCall(_, _, expressions)),
            )) => {
                // We can safely unwrap the first expression because this function
                // will only be called on a session source that has just had an
                // `inspectoor` variable appended to it.
                Some(expressions.first().unwrap())
            }
            _ => None,
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
                Paint::yellow(tokens.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")),
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

// =============================================
// Modified from
// [soli](https://github.com/jpopesculian/soli)
// =============================================

#[derive(Debug, Clone)]
enum Type {
    /// (type)
    Builtin(ParamType),

    /// (type)
    Array(Box<Type>),

    /// (type, length)
    FixedArray(Box<Type>, usize),

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
            pt::Expression::This(_) => Some(Self::Custom(vec!["this".to_string()])),

            // array
            pt::Expression::ArraySubscript(_, expr, num) => {
                Self::from_expression(expr).and_then(|ty| {
                    let ty = Box::new(ty);
                    let num = num.as_deref().and_then(parse_number_literal);
                    if let Some(num) = num {
                        // overflow check
                        if num > U256::from(usize::MAX) {
                            None
                        } else {
                            Some(Self::FixedArray(ty, num.as_usize()))
                        }
                    } else {
                        Some(Self::Array(ty))
                    }
                })
            }
            // TODO: offset and length do not get encoded when inspecting, so this always throws
            // pt::Expression::ArrayLiteral(_, values) => {
            //     values.first().and_then(Self::from_expression).map(|ty| {
            //         Self::Array(Box::new(ty))
            //     })
            // }

            // <lhs>.<rhs>
            pt::Expression::MemberAccess(_, lhs, rhs) => {
                Self::from_expression(lhs).map(|lhs| {
                    Self::Access(Box::new(lhs), rhs.name.clone())
                })
            }

            // <inner>
            pt::Expression::Parenthesis(_, inner) |      // (<inner>)
            pt::Expression::New(_, inner) |              // new <inner>
            pt::Expression::UnaryPlus(_, inner) |        // +<inner>
            pt::Expression::Unit(_, inner, _) |          // <inner> *unit*
            // ops
            pt::Expression::Complement(_, inner) |       // ~<inner>
            pt::Expression::ArraySlice(_, inner, _, _)   // <inner>[*start*:*end*]
            // assign ops
            // TODO: If this returns Some and gets "inspected", the assignment does not happen
            // pt::Expression::PreDecrement(_, inner) |     // --<inner>
            // pt::Expression::PostDecrement(_, inner) |    // <inner>--
            // pt::Expression::PreIncrement(_, inner) |     // ++<inner>
            // pt::Expression::PostIncrement(_, inner)      // <inner>++
            => Self::from_expression(inner),

            // *condition* ? <if_true> : <if_false>
            pt::Expression::ConditionalOperator(_, _, if_true, if_false) => {
                Self::from_expression(if_true).or_else(|| Self::from_expression(if_false))
            }

            // address
            pt::Expression::AddressLiteral(_, _) => Some(Self::Builtin(ParamType::Address)),

            // uint and int
            // invert
            pt::Expression::UnaryMinus(_, inner) => Self::from_expression(inner).map(Self::invert_sign),

            // assume uint
            // TODO: Perform operations to find negative numbers
            pt::Expression::Add(_, _, _) |
            pt::Expression::Subtract(_, _, _) |
            pt::Expression::Multiply(_, _, _) |
            pt::Expression::Divide(_, _, _) |
            pt::Expression::Modulo(_, _, _) |
            pt::Expression::Power(_, _, _) |
            pt::Expression::BitwiseOr(_, _, _) |
            pt::Expression::BitwiseAnd(_, _, _) |
            pt::Expression::BitwiseXor(_, _, _) |
            pt::Expression::ShiftRight(_, _, _) |
            pt::Expression::ShiftLeft(_, _, _) |
            pt::Expression::NumberLiteral(_, _, _) |
            pt::Expression::HexNumberLiteral(_, _) => Some(Self::Builtin(ParamType::Uint(256))),

            // TODO
            pt::Expression::RationalNumberLiteral(_, _, _, _) => {
                Some(Self::Builtin(ParamType::Uint(256)))
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
            pt::Expression::Not(_, _) => Some(Self::Builtin(ParamType::Bool)),

            // string
            pt::Expression::StringLiteral(_) => Some(Self::Builtin(ParamType::String)),

            // bytes
            pt::Expression::HexLiteral(_) => Some(Self::Builtin(ParamType::Bytes)),

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

            _ => None,
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
                Self::Builtin(ParamType::Address)
            }
            pt::Type::Bool => Self::Builtin(ParamType::Bool),
            pt::Type::String => Self::Builtin(ParamType::String),
            pt::Type::Int(size) => Self::Builtin(ParamType::Int(*size as usize)),
            pt::Type::Uint(size) => Self::Builtin(ParamType::Uint(*size as usize)),
            pt::Type::Bytes(size) => Self::Builtin(ParamType::FixedBytes(*size as usize)),
            pt::Type::DynamicBytes => Self::Builtin(ParamType::Bytes),
            pt::Type::Mapping(_, _, right) => Self::from_expression(right)?,
            pt::Type::Function { params, returns, .. } => {
                let params = map_parameters(params);
                let returns = returns
                    .as_ref()
                    .map(|(returns, _)| map_parameters(returns))
                    .unwrap_or_default();
                Self::Function(
                    Box::new(Type::Custom(vec!["__fn_type__".to_string()])),
                    params,
                    returns,
                )
            }
            // TODO
            pt::Type::Rational => return None,
        };
        Some(ty)
    }

    /// Handle special expressions like global variables and methods
    fn map_special(self) -> Self {
        if !matches!(self, Self::Function(_, _, _) | Self::Access(_, _) | Self::Custom(_),) {
            return self
        }

        let mut types = Vec::with_capacity(5);
        let mut args = None;
        self.recurse(&mut types, &mut args);
        eprintln!("{types:?}");
        eprintln!("{args:?}");

        if !(1..=2).contains(&types.len()) {
            return self
        }

        let this = match types.len() {
            1 => {
                let name = types.pop().unwrap();
                match name.as_str() {
                    "gasleft" | "addmod" | "mulmod" => Some(ParamType::Uint(256)),
                    "keccak256" | "sha256" | "blockhash" => Some(ParamType::FixedBytes(32)),
                    "ripemd160" => Some(ParamType::FixedBytes(20)),
                    "ecrecover" => Some(ParamType::Address),
                    _ => None,
                }
            }
            2 => {
                let name = types.pop().unwrap();
                let access_s = types.pop().unwrap();
                let access = access_s.as_str();
                match name.as_str() {
                    "block" => match access {
                        "coinbase" => Some(ParamType::Address),
                        _ => Some(ParamType::Uint(256)),
                    },
                    "msg" => match access {
                        "data" => Some(ParamType::Bytes),
                        "sender" => Some(ParamType::Address),
                        "sig" => Some(ParamType::FixedBytes(4)),
                        "value" => Some(ParamType::Uint(256)),
                        _ => None,
                    },
                    "tx" => match access {
                        "gasprice" => Some(ParamType::Uint(256)),
                        "origin" => Some(ParamType::Address),
                        _ => None,
                    },
                    "abi" => {
                        if access.starts_with("decode") {
                            // TODO: Fill value types
                            Some(ParamType::Tuple(vec![ParamType::Uint(256)]))
                        } else {
                            Some(ParamType::Bytes)
                        }
                    }
                    "address" => match access {
                        "balance" => Some(ParamType::Uint(256)),
                        "code" => Some(ParamType::Bytes),
                        "codehash" => Some(ParamType::FixedBytes(32)),
                        _ => None,
                    },
                    "type" => match access {
                        "name" => Some(ParamType::String),
                        "creationCode" | "runtimeCode" => Some(ParamType::Bytes),
                        "interfaceId" => Some(ParamType::FixedBytes(4)),
                        "min" | "max" => Some(ParamType::Uint(256)),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => unreachable!(),
        };
        this.map(Self::Builtin).unwrap_or(self)
    }

    /// Recurses over itself, appending all the idents and function arguments in the order that they
    /// are found
    fn recurse(&self, types: &mut Vec<String>, args: &mut Option<Vec<Option<Type>>>) {
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
            let Some(intermediate_contract) = intermediate.intermediate_contracts.get(&contract_name) else {
                eyre::bail!("Could not find intermediate contract!")
            };
            let cur_type = custom_type.last().ok_or(eyre::eyre!(""))?;

            if let Some(func) = intermediate_contract.function_definitions.get(cur_type) {
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

                Ok(Type::ethabi(return_ty, intermediate))
            } else if let Some(var_def) = intermediate_contract.variable_definitions.get(cur_type) {
                match &var_def.ty {
                    // If we're here, we're indexing an array within this contract:
                    // use the inner type
                    pt::Expression::ArraySubscript(_, expr, _) => {
                        Ok(Type::ethabi(expr, intermediate))
                    }
                    // Custom variable handling
                    pt::Expression::Variable(pt::Identifier { loc: _, name }) => {
                        if intermediate_contract.struct_definitions.get(name).is_some() {
                            // A struct type was found: we set the custom type to just the struct's
                            // name and re-enter the recursion one last time.
                            custom_type.clear();
                            custom_type.push(name.clone());

                            Self::infer_custom_type(intermediate, custom_type, Some(contract_name))
                        } else if intermediate.intermediate_contracts.get(name).is_some() {
                            if custom_type.len() > 1 {
                                // There is still some recursing left to do: jump into the contract.
                                custom_type.pop();
                                Self::infer_custom_type(
                                    intermediate,
                                    custom_type,
                                    Some(name.clone()),
                                )
                            } else {
                                // We have no types left to recurse: return the address of the
                                // contract.
                                Ok(Some(ParamType::Address))
                            }
                        } else {
                            eyre::bail!("Could not infer variable type")
                        }
                    }
                    ty => Ok(Type::ethabi(ty, intermediate)),
                }
            } else if let Some(struct_def) = intermediate_contract.struct_definitions.get(cur_type)
            {
                let inner_types = struct_def
                    .fields
                    .iter()
                    .map(|var| {
                        Type::ethabi(&var.ty, intermediate)
                            .ok_or_else(|| eyre::eyre!("Struct `{cur_type}` has invalid fields"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Some(ParamType::Tuple(inner_types)))
            } else {
                eyre::bail!("Could not find function definitions for contract!")
            }
        } else {
            // Check if the custom type is a variable or function within the REPL contract before
            // anything. If it is, we can stop here.
            match Self::infer_custom_type(intermediate, custom_type, Some("REPL".into())) {
                Ok(res) => return Ok(res),
                _ => {}
            }

            // Check if the first element of the custom type is a known contract. If it is, begin
            // our recursion on on that contract's definitions.
            let contract = intermediate.intermediate_contracts.get(custom_type.last().unwrap());
            if contract.is_some() {
                let contract_name = custom_type.pop();
                return Self::infer_custom_type(intermediate, custom_type, contract_name)
            }

            // If the first element of the custom type is a variable within the REPL contract,
            // that variable could be a contract itself, so we recurse back into this function
            // with a contract name set.
            //
            // Otherwise, we return the type of the expression, as defined in
            // `Type::from_expression`.
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
                    expr => return Ok(Type::ethabi(expr, intermediate)),
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
            Self::Function(name, _, _) => match *name {
                Type::Builtin(ty) => Some(ty),
                _ => None,
            },
            Self::Access(_, _) => None,
            Self::Custom(mut types) => {
                // Cover any local non-state-modifying function call expressions
                match Self::infer_custom_type(intermediate, &mut types, None) {
                    Ok(opt @ Some(_)) => return opt,
                    Ok(None) => {}
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

    /// Equivalent to `Type::from_expression` + `Type::map_special` + `Type::try_as_ethabi)`
    fn ethabi(expr: &pt::Expression, intermediate: &IntermediateOutput) -> Option<ParamType> {
        Self::from_expression(expr)
            .map(Self::map_special)
            .and_then(|ty| ty.try_as_ethabi(intermediate))
    }

    /// Inverts Int to Uint and viceversa.
    fn invert_sign(self) -> Self {
        match self {
            Self::Builtin(ParamType::Uint(n)) => Self::Builtin(ParamType::Int(n)),
            Self::Builtin(ParamType::Int(n)) => Self::Builtin(ParamType::Uint(n)),
            x => x,
        }
    }
}

fn map_parameters(params: &Vec<(pt::Loc, Option<pt::Parameter>)>) -> Vec<Option<Type>> {
    params
        .iter()
        .map(|(_, param)| {
            param.as_ref().and_then(|param| match &param.ty {
                pt::Expression::Type(_, ty) => Type::from_type(ty),
                _ => None, // Should not happen
            })
        })
        .collect()
}

fn parse_number_literal(expr: &pt::Expression) -> Option<U256> {
    match expr {
        pt::Expression::NumberLiteral(_, num, exp) => {
            let num = U256::from_dec_str(num).unwrap_or(U256::zero());
            let exp = exp.parse().unwrap_or(0u32);
            if exp > 77 {
                None
            } else {
                Some(num * U256::from(10usize.pow(exp)))
            }
        }
        pt::Expression::HexNumberLiteral(_, num) => num.parse::<U256>().ok(),
        // TODO
        pt::Expression::RationalNumberLiteral(_, _, _, _) => None,

        pt::Expression::Unit(_, expr, unit) => {
            parse_number_literal(expr).map(|x| x * unit_multiplier(unit))
        }

        _ => None,
    }
}

#[inline]
const fn unit_multiplier(unit: &pt::Unit) -> usize {
    use pt::Unit::*;
    match unit {
        Seconds(_) => 1,
        Minutes(_) => 60,
        Hours(_) => 60 * 60,
        Days(_) => 60 * 60 * 24,
        Weeks(_) => 60 * 60 * 24 * 7,
        Wei(_) => 1,
        Gwei(_) => 10_usize.pow(9),
        Ether(_) => 10_usize.pow(18),
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
