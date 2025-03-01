//! [`Type`] expansion.

use super::ExpCtxt;
use ast::{Item, Parameters, Spanned, Type, TypeArray};
use proc_macro2::{Ident, Literal, Span, TokenStream};
use proc_macro_error2::{abort, emit_error};
use quote::{quote_spanned, ToTokens};
use std::{fmt, num::NonZeroU16};

const MAX_SUPPORTED_ARRAY_LEN: usize = 32;
const MAX_SUPPORTED_TUPLE_LEN: usize = 12;

impl ExpCtxt<'_> {
    /// Expands a single [`Type`] recursively to its `alloy_sol_types::sol_data`
    /// equivalent.
    pub fn expand_type(&self, ty: &Type) -> TokenStream {
        let mut tokens = TokenStream::new();
        self.expand_type_to(ty, &mut tokens);
        tokens
    }

    /// Expands a single [`Type`] recursively to its Rust type equivalent.
    ///
    /// This is the same as `<#expand_type(ty) as SolType>::RustType`, but generates
    /// nicer code for documentation and IDE/LSP support when the type is not
    /// ambiguous.
    pub fn expand_rust_type(&self, ty: &Type) -> TokenStream {
        let mut tokens = TokenStream::new();
        self.expand_rust_type_to(ty, &mut tokens);
        tokens
    }

    /// Expands a single [`Type`] recursively to its `alloy_sol_types::sol_data` equivalent into the
    /// given buffer.
    ///
    /// See [`expand_type`](Self::expand_type) for more information.
    pub fn expand_type_to(&self, ty: &Type, tokens: &mut TokenStream) {
        let alloy_sol_types = &self.crates.sol_types;
        let tts = match *ty {
            Type::Address(span, _) => quote_spanned! {span=> #alloy_sol_types::sol_data::Address },
            Type::Bool(span) => quote_spanned! {span=> #alloy_sol_types::sol_data::Bool },
            Type::String(span) => quote_spanned! {span=> #alloy_sol_types::sol_data::String },
            Type::Bytes(span) => quote_spanned! {span=> #alloy_sol_types::sol_data::Bytes },

            Type::FixedBytes(span, size) => {
                assert!(size.get() <= 32);
                let size = Literal::u16_unsuffixed(size.get());
                quote_spanned! {span=> #alloy_sol_types::sol_data::FixedBytes<#size> }
            }
            Type::Int(span, size) | Type::Uint(span, size) => {
                let name = match ty {
                    Type::Int(..) => "Int",
                    Type::Uint(..) => "Uint",
                    _ => unreachable!(),
                };
                let name = Ident::new(name, span);

                let size = size.map_or(256, NonZeroU16::get);
                assert!(size <= 256 && size % 8 == 0);
                let size = Literal::u16_unsuffixed(size);

                quote_spanned! {span=> #alloy_sol_types::sol_data::#name<#size> }
            }

            Type::Tuple(ref tuple) => {
                return tuple.paren_token.surround(tokens, |tokens| {
                    for pair in tuple.types.pairs() {
                        let (ty, comma) = pair.into_tuple();
                        self.expand_type_to(ty, tokens);
                        comma.to_tokens(tokens);
                    }
                })
            }
            Type::Array(ref array) => {
                let ty = self.expand_type(&array.ty);
                let span = array.span();
                if let Some(size) = self.eval_array_size(array) {
                    quote_spanned! {span=> #alloy_sol_types::sol_data::FixedArray<#ty, #size> }
                } else {
                    quote_spanned! {span=> #alloy_sol_types::sol_data::Array<#ty> }
                }
            }
            Type::Function(ref function) => quote_spanned! {function.span()=>
                #alloy_sol_types::sol_data::Function
            },
            Type::Mapping(ref mapping) => quote_spanned! {mapping.span()=>
                ::core::compile_error!("Mapping types are not supported here")
            },

            Type::Custom(ref custom) => {
                if let Some(Item::Contract(c)) = self.try_item(custom) {
                    quote_spanned! {c.span()=> #alloy_sol_types::sol_data::Address }
                } else {
                    let segments = custom.iter();
                    quote_spanned! {custom.span()=> #(#segments)::* }
                }
            }
        };
        tokens.extend(tts);
    }

    // IMPORTANT: Keep in sync with `sol-types/src/types/data_type.rs`
    /// Expands a single [`Type`] recursively to its Rust type equivalent into the given buffer.
    ///
    /// See [`expand_rust_type`](Self::expand_rust_type) for more information.
    pub(crate) fn expand_rust_type_to(&self, ty: &Type, tokens: &mut TokenStream) {
        let alloy_sol_types = &self.crates.sol_types;
        let tts = match *ty {
            Type::Address(span, _) => quote_spanned! {span=> #alloy_sol_types::private::Address },
            Type::Bool(span) => return Ident::new("bool", span).to_tokens(tokens),
            Type::String(span) => quote_spanned! {span=> #alloy_sol_types::private::String },
            Type::Bytes(span) => quote_spanned! {span=> #alloy_sol_types::private::Bytes },

            Type::FixedBytes(span, size) => {
                assert!(size.get() <= 32);
                let size = Literal::u16_unsuffixed(size.get());
                quote_spanned! {span=> #alloy_sol_types::private::FixedBytes<#size> }
            }
            Type::Int(span, size) | Type::Uint(span, size) => {
                let size = size.map_or(256, NonZeroU16::get);
                let primitive = matches!(size, 8 | 16 | 32 | 64 | 128);
                if primitive {
                    let prefix = match ty {
                        Type::Int(..) => "i",
                        Type::Uint(..) => "u",
                        _ => unreachable!(),
                    };
                    return Ident::new(&format!("{prefix}{size}"), span).to_tokens(tokens);
                }
                let prefix = match ty {
                    Type::Int(..) => "I",
                    Type::Uint(..) => "U",
                    _ => unreachable!(),
                };
                let name = Ident::new(&format!("{prefix}{size}"), span);
                quote_spanned! {span=> #alloy_sol_types::private::primitives::aliases::#name }
            }

            Type::Tuple(ref tuple) => {
                return tuple.paren_token.surround(tokens, |tokens| {
                    for pair in tuple.types.pairs() {
                        let (ty, comma) = pair.into_tuple();
                        self.expand_rust_type_to(ty, tokens);
                        comma.to_tokens(tokens);
                    }
                })
            }
            Type::Array(ref array) => {
                let ty = self.expand_rust_type(&array.ty);
                let span = array.span();
                if let Some(size) = self.eval_array_size(array) {
                    quote_spanned! {span=> [#ty; #size] }
                } else {
                    quote_spanned! {span=> #alloy_sol_types::private::Vec<#ty> }
                }
            }
            Type::Function(ref function) => quote_spanned! {function.span()=>
                #alloy_sol_types::private::Function
            },
            Type::Mapping(ref mapping) => quote_spanned! {mapping.span()=>
                ::core::compile_error!("Mapping types are not supported here")
            },

            // Exhaustive fallback to `SolType::RustType`
            Type::Custom(_) => {
                let span = ty.span();
                let ty = self.expand_type(ty);
                quote_spanned! {span=> <#ty as #alloy_sol_types::SolType>::RustType }
            }
        };
        tokens.extend(tts);
    }

    /// Calculates the base ABI-encoded size of the given parameters in bytes.
    ///
    /// See [`type_base_data_size`] for more information.
    pub(crate) fn params_base_data_size<P>(&self, params: &Parameters<P>) -> usize {
        params.iter().map(|param| self.type_base_data_size(&param.ty)).sum()
    }

    /// Recursively calculates the base ABI-encoded size of the given parameter
    /// in bytes.
    ///
    /// That is, the minimum number of bytes required to encode `self` without
    /// any dynamic data.
    pub(crate) fn type_base_data_size(&self, ty: &Type) -> usize {
        match ty {
            // static types: 1 word
            Type::Address(..)
            | Type::Bool(_)
            | Type::Int(..)
            | Type::Uint(..)
            | Type::FixedBytes(..)
            | Type::Function(_) => 32,

            // dynamic types: 1 offset word, 1 length word
            Type::String(_) | Type::Bytes(_) | Type::Array(TypeArray { size: None, .. }) => 64,

            // fixed array: size * encoded size
            Type::Array(a @ TypeArray { ty: inner, size: Some(_), .. }) => {
                let Some(size) = self.eval_array_size(a) else { return 0 };
                self.type_base_data_size(inner).checked_mul(size).unwrap_or(0)
            }

            // tuple: sum of encoded sizes
            Type::Tuple(tuple) => tuple.types.iter().map(|ty| self.type_base_data_size(ty)).sum(),

            Type::Custom(name) => match self.try_item(name) {
                Some(Item::Contract(_)) | Some(Item::Enum(_)) => 32,
                Some(Item::Error(error)) => {
                    error.parameters.types().map(|ty| self.type_base_data_size(ty)).sum()
                }
                Some(Item::Event(event)) => {
                    event.parameters.iter().map(|p| self.type_base_data_size(&p.ty)).sum()
                }
                Some(Item::Struct(strukt)) => {
                    strukt.fields.types().map(|ty| self.type_base_data_size(ty)).sum()
                }
                Some(Item::Udt(udt)) => self.type_base_data_size(&udt.ty),
                Some(item) => abort!(item.span(), "Invalid type in struct field: {:?}", item),
                None => 0,
            },

            // not applicable
            Type::Mapping(_) => 0,
        }
    }

    /// Returns whether the given type can derive the [`Default`] trait.
    pub(crate) fn can_derive_default(&self, ty: &Type) -> bool {
        match ty {
            Type::Array(a) => {
                self.eval_array_size(a).map_or(true, |sz| sz <= MAX_SUPPORTED_ARRAY_LEN)
                    && self.can_derive_default(&a.ty)
            }
            Type::Tuple(tuple) => {
                if tuple.types.len() > MAX_SUPPORTED_TUPLE_LEN {
                    false
                } else {
                    tuple.types.iter().all(|ty| self.can_derive_default(ty))
                }
            }

            Type::Custom(name) => match self.try_item(name) {
                Some(Item::Contract(_)) => true,
                Some(Item::Enum(_)) => false,
                Some(Item::Error(error)) => {
                    error.parameters.types().all(|ty| self.can_derive_default(ty))
                }
                Some(Item::Event(event)) => {
                    event.parameters.iter().all(|p| self.can_derive_default(&p.ty))
                }
                Some(Item::Struct(strukt)) => {
                    strukt.fields.types().all(|ty| self.can_derive_default(ty))
                }
                Some(Item::Udt(udt)) => self.can_derive_default(&udt.ty),
                Some(item) => abort!(item.span(), "Invalid type in struct field: {:?}", item),
                _ => false,
            },

            _ => true,
        }
    }

    /// Returns whether the given type can derive the builtin traits listed in
    /// `ExprCtxt::derives`, minus `Default`.
    pub(crate) fn can_derive_builtin_traits(&self, ty: &Type) -> bool {
        match ty {
            Type::Array(a) => self.can_derive_builtin_traits(&a.ty),
            Type::Tuple(tuple) => {
                if tuple.types.len() > MAX_SUPPORTED_TUPLE_LEN {
                    false
                } else {
                    tuple.types.iter().all(|ty| self.can_derive_builtin_traits(ty))
                }
            }

            Type::Custom(name) => match self.try_item(name) {
                Some(Item::Contract(_)) | Some(Item::Enum(_)) => true,
                Some(Item::Error(error)) => {
                    error.parameters.types().all(|ty| self.can_derive_builtin_traits(ty))
                }
                Some(Item::Event(event)) => {
                    event.parameters.iter().all(|p| self.can_derive_builtin_traits(&p.ty))
                }
                Some(Item::Struct(strukt)) => {
                    strukt.fields.types().all(|ty| self.can_derive_builtin_traits(ty))
                }
                Some(Item::Udt(udt)) => self.can_derive_builtin_traits(&udt.ty),
                Some(item) => abort!(item.span(), "Invalid type in struct field: {:?}", item),
                _ => false,
            },

            _ => true,
        }
    }

    /// Evaluates the size of the given array type.
    pub fn eval_array_size(&self, array: &TypeArray) -> Option<ArraySize> {
        let size = array.size.as_deref()?;
        ArraySizeEvaluator::new(self).eval(size)
    }
}

type ArraySize = usize;

struct ArraySizeEvaluator<'a> {
    cx: &'a ExpCtxt<'a>,
    depth: usize,
}

impl<'a> ArraySizeEvaluator<'a> {
    fn new(cx: &'a ExpCtxt<'a>) -> Self {
        Self { cx, depth: 0 }
    }

    fn eval(&mut self, expr: &ast::Expr) -> Option<ArraySize> {
        match self.try_eval(expr) {
            Ok(value) => Some(value),
            Err(err) => {
                emit_error!(
                    expr.span(), "evaluation of constant value failed";
                    note = err.span() => err.kind.msg()
                );
                None
            }
        }
    }

    fn try_eval(&mut self, expr: &ast::Expr) -> Result<ArraySize, EvalError> {
        self.depth += 1;
        if self.depth > 32 {
            return Err(EvalErrorKind::RecursionLimitReached.spanned(expr.span()));
        }
        let mut r = self.try_eval_expr(expr);
        if let Err(e) = &mut r {
            if e.span.is_none() {
                e.span = Some(expr.span());
            }
        }
        self.depth -= 1;
        r
    }

    fn try_eval_expr(&mut self, expr: &ast::Expr) -> Result<ArraySize, EvalError> {
        let expr = expr.peel_parens();
        match expr {
            ast::Expr::Lit(ast::Lit::Number(ast::LitNumber::Int(n))) => {
                n.base10_digits().parse::<ArraySize>().map_err(|_| EE::ParseInt.into())
            }
            ast::Expr::Binary(bin) => {
                let lhs = self.try_eval(&bin.left)?;
                let rhs = self.try_eval(&bin.right)?;
                self.eval_binop(bin.op, lhs, rhs)
            }
            ast::Expr::Ident(ident) => {
                let name = ast::sol_path![ident.clone()];
                let Some(item) = self.cx.try_item(&name) else {
                    eprintln!("{}", std::backtrace::Backtrace::force_capture());
                    eprintln!("{:#?}", self.cx.all_items);
                    return Err(EE::CouldNotResolve.into());
                };
                let ast::Item::Variable(var) = item else {
                    return Err(EE::NonConstantVar.into());
                };
                if !var.attributes.has_constant() {
                    return Err(EE::NonConstantVar.into());
                }
                let Some((_, expr)) = var.initializer.as_ref() else {
                    return Err(EE::NonConstantVar.into());
                };
                self.try_eval(expr)
            }
            ast::Expr::LitDenominated(ast::LitDenominated {
                number: ast::LitNumber::Int(n),
                denom,
            }) => {
                let n = n.base10_digits().parse::<ArraySize>().map_err(|_| EE::ParseInt)?;
                let Ok(denom) = denom.value().try_into() else {
                    return Err(EE::IntTooBig.into());
                };
                n.checked_mul(denom).ok_or_else(|| EE::ArithmeticOverflow.into())
            }
            ast::Expr::Unary(unary) => {
                let value = self.try_eval(&unary.expr)?;
                self.eval_unop(unary.op, value)
            }
            _ => Err(EE::UnsupportedExpr.into()),
        }
    }

    fn eval_binop(
        &mut self,
        bin: ast::BinOp,
        lhs: ArraySize,
        rhs: ArraySize,
    ) -> Result<ArraySize, EvalError> {
        match bin {
            ast::BinOp::Shr(..) => rhs
                .try_into()
                .ok()
                .and_then(|rhs| lhs.checked_shr(rhs))
                .ok_or_else(|| EE::ArithmeticOverflow.into()),
            ast::BinOp::Shl(..) => rhs
                .try_into()
                .ok()
                .and_then(|rhs| lhs.checked_shl(rhs))
                .ok_or_else(|| EE::ArithmeticOverflow.into()),
            ast::BinOp::BitAnd(..) => Ok(lhs & rhs),
            ast::BinOp::BitOr(..) => Ok(lhs | rhs),
            ast::BinOp::BitXor(..) => Ok(lhs ^ rhs),
            ast::BinOp::Add(..) => {
                lhs.checked_add(rhs).ok_or_else(|| EE::ArithmeticOverflow.into())
            }
            ast::BinOp::Sub(..) => {
                lhs.checked_sub(rhs).ok_or_else(|| EE::ArithmeticOverflow.into())
            }
            ast::BinOp::Pow(..) => rhs
                .try_into()
                .ok()
                .and_then(|rhs| lhs.checked_pow(rhs))
                .ok_or_else(|| EE::ArithmeticOverflow.into()),
            ast::BinOp::Mul(..) => {
                lhs.checked_mul(rhs).ok_or_else(|| EE::ArithmeticOverflow.into())
            }
            ast::BinOp::Div(..) => lhs.checked_div(rhs).ok_or_else(|| EE::DivisionByZero.into()),
            ast::BinOp::Rem(..) => lhs.checked_div(rhs).ok_or_else(|| EE::DivisionByZero.into()),
            _ => Err(EE::UnsupportedExpr.into()),
        }
    }

    fn eval_unop(&mut self, unop: ast::UnOp, value: ArraySize) -> Result<ArraySize, EvalError> {
        match unop {
            ast::UnOp::Neg(..) => value.checked_neg().ok_or_else(|| EE::ArithmeticOverflow.into()),
            ast::UnOp::BitNot(..) | ast::UnOp::Not(..) => Ok(!value),
            _ => Err(EE::UnsupportedUnaryOp.into()),
        }
    }
}

struct EvalError {
    kind: EvalErrorKind,
    span: Option<Span>,
}

impl From<EvalErrorKind> for EvalError {
    fn from(kind: EvalErrorKind) -> Self {
        Self { kind, span: None }
    }
}

impl EvalError {
    fn span(&self) -> Span {
        self.span.unwrap_or_else(Span::call_site)
    }
}

enum EvalErrorKind {
    RecursionLimitReached,
    ArithmeticOverflow,
    ParseInt,
    IntTooBig,
    DivisionByZero,
    UnsupportedUnaryOp,
    UnsupportedExpr,
    CouldNotResolve,
    NonConstantVar,
}
use EvalErrorKind as EE;

impl EvalErrorKind {
    fn spanned(self, span: Span) -> EvalError {
        EvalError { kind: self, span: Some(span) }
    }

    fn msg(&self) -> &'static str {
        match self {
            Self::RecursionLimitReached => "recursion limit reached",
            Self::ArithmeticOverflow => "arithmetic overflow",
            Self::ParseInt => "failed to parse integer",
            Self::IntTooBig => "integer value is too big",
            Self::DivisionByZero => "division by zero",
            Self::UnsupportedUnaryOp => "unsupported unary operation",
            Self::UnsupportedExpr => "unsupported expression",
            Self::CouldNotResolve => "could not resolve identifier",
            Self::NonConstantVar => "only constant variables are allowed",
        }
    }
}

/// Implements [`fmt::Display`] which formats a [`Type`] to its canonical
/// representation. This is then used in function, error, and event selector
/// generation.
pub(crate) struct TypePrinter<'ast> {
    cx: &'ast ExpCtxt<'ast>,
    ty: &'ast Type,
}

impl<'ast> TypePrinter<'ast> {
    pub(crate) fn new(cx: &'ast ExpCtxt<'ast>, ty: &'ast Type) -> Self {
        Self { cx, ty }
    }
}

impl fmt::Display for TypePrinter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ty {
            Type::Int(_, None) => f.write_str("int256"),
            Type::Uint(_, None) => f.write_str("uint256"),

            Type::Array(array) => {
                Self::new(self.cx, &array.ty).fmt(f)?;
                f.write_str("[")?;
                if let Some(size) = self.cx.eval_array_size(array) {
                    size.fmt(f)?;
                }
                f.write_str("]")
            }
            Type::Tuple(tuple) => {
                f.write_str("(")?;
                for (i, ty) in tuple.types.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    Self::new(self.cx, ty).fmt(f)?;
                }
                f.write_str(")")
            }

            Type::Custom(name) => Self::new(self.cx, self.cx.custom_type(name)).fmt(f),

            ty => ty.fmt(f),
        }
    }
}
