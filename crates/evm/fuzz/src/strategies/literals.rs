use alloy_dyn_abi::DynSolType;
use alloy_primitives::{
    B256, Bytes, I256, U256, keccak256,
    map::{B256IndexSet, HashMap, IndexSet},
};
use foundry_common::Analysis;
use foundry_compilers::ProjectPathsConfig;
use solar::{
    ast::{
        self,
        BinOpKind::{Add, BitAnd, BitOr, BitXor, Div, Mul, Pow, Rem, Shl, Shr, Sub},
        Visit,
    },
    interface::{Span, source_map::FileName},
};
use std::{
    cell::RefCell,
    ops::ControlFlow,
    sync::{Arc, OnceLock},
};

/// Maximum nesting depth [`LiteralsCollector::eval`] recurses into, to bound stack usage.
const MAX_FOLD_DEPTH: usize = 128;

#[derive(Clone, Debug)]
pub struct LiteralsDictionary {
    maps: Arc<OnceLock<LiteralMaps>>,
}

impl Default for LiteralsDictionary {
    fn default() -> Self {
        Self::new(None, None, usize::MAX)
    }
}

impl LiteralsDictionary {
    pub fn new(
        analysis: Option<Analysis>,
        paths_config: Option<ProjectPathsConfig>,
        max_values: usize,
    ) -> Self {
        let maps = Arc::new(OnceLock::<LiteralMaps>::new());
        if let Some(analysis) = analysis
            && max_values > 0
        {
            let maps = maps.clone();
            // This can't be done in a rayon task (including inside of `get`) because it can cause a
            // deadlock, since internally `solar` also uses rayon.
            let _ = std::thread::Builder::new().name("literal-collector".into()).spawn(move || {
                let _ = maps.get_or_init(|| {
                    let literals =
                        LiteralsCollector::process(&analysis, paths_config.as_ref(), max_values);
                    debug!(
                        words = literals.words.values().map(|set| set.len()).sum::<usize>(),
                        strings = literals.strings.len(),
                        bytes = literals.bytes.len(),
                        "collected source code literals for fuzz dictionary"
                    );
                    literals
                });
            });
        } else {
            maps.set(Default::default()).unwrap();
        }
        Self { maps }
    }

    /// Returns a reference to the `LiteralMaps`.
    pub fn get(&self) -> &LiteralMaps {
        self.maps.wait()
    }

    /// Test-only helper to seed the dictionary with literal values.
    #[cfg(test)]
    pub(crate) fn set(&mut self, map: super::LiteralMaps) {
        self.maps = Arc::new(OnceLock::new());
        self.maps.set(map).unwrap();
    }
}

#[derive(Debug, Default)]
pub struct LiteralMaps {
    pub words: HashMap<DynSolType, B256IndexSet>,
    pub strings: IndexSet<String>,
    pub bytes: IndexSet<Bytes>,
}

#[derive(Debug, Default)]
pub struct LiteralsCollector {
    max_values: usize,
    total_values: usize,
    output: LiteralMaps,
    /// Memoizes [`Self::eval`] results by expression span so overlapping subtrees are folded once.
    eval_cache: RefCell<HashMap<Span, Option<Num>>>,
}

impl LiteralsCollector {
    fn new(max_values: usize) -> Self {
        Self { max_values, ..Default::default() }
    }

    pub fn process(
        analysis: &Analysis,
        paths_config: Option<&ProjectPathsConfig>,
        max_values: usize,
    ) -> LiteralMaps {
        analysis.enter(|compiler| {
            let mut literals_collector = Self::new(max_values);
            for source in compiler.sources().iter() {
                // Ignore scripts, and libs
                if let Some(paths) = paths_config
                    && let FileName::Real(source_path) = &source.file.name
                    && !(source_path.starts_with(&paths.sources) || paths.is_test(source_path))
                {
                    continue;
                }

                if let Some(ast) = &source.ast
                    && literals_collector.visit_source_unit(ast).is_break()
                {
                    break;
                }
            }

            literals_collector.output
        })
    }

    /// Inserts a single word value under the given type, respecting the value limit.
    fn insert_word(&mut self, ty: DynSolType, word: B256) {
        if self.total_values < self.max_values
            && self.output.words.entry(ty).or_default().insert(word)
        {
            self.total_values += 1;
        }
    }

    /// Inserts a string value, respecting the value limit.
    fn insert_string(&mut self, s: String) {
        if self.total_values < self.max_values && self.output.strings.insert(s) {
            self.total_values += 1;
        }
    }

    /// Inserts a raw bytes value, respecting the value limit.
    fn insert_bytes(&mut self, bytes: Bytes) {
        if self.total_values < self.max_values && self.output.bytes.insert(bytes) {
            self.total_values += 1;
        }
    }

    /// Seeds an unsigned value under all `uintN` sizes that can represent it.
    fn seed_uint(&mut self, value: U256) {
        let word = B256::from(value);
        for bits in [8, 16, 32, 64, 128, 256] {
            if can_fit_uint(value, bits) {
                self.insert_word(DynSolType::Uint(bits), word);
            }
        }
    }

    /// Seeds a signed value under all `intN` sizes that can represent it.
    fn seed_int(&mut self, value: I256) {
        let word = B256::from(value.into_raw());
        for bits in [8, 16, 32, 64, 128, 256] {
            if can_fit_int(value, bits) {
                self.insert_word(DynSolType::Int(bits), word);
            }
        }
    }

    /// Seeds a folded value: under its exact type when it carries a width, and (for integers) under
    /// every smaller size that can represent it.
    fn seed_num(&mut self, value: Num) {
        match value {
            Num::Int { raw, signed: false, width } => {
                if let Some(bits) = width {
                    self.insert_word(DynSolType::Uint(bits), B256::from(raw));
                }
                self.seed_uint(raw);
            }
            Num::Int { signed: true, width, .. } => {
                let i = value.to_i256().expect("signed values always convert to I256");
                if let Some(bits) = width {
                    self.insert_word(DynSolType::Int(bits), B256::from(i.into_raw()));
                }
                self.seed_int(i);
            }
            // `bytesN` is left-aligned in the word, while integers are right-aligned.
            Num::Bytes { raw, n } => {
                let word = if n >= 32 { raw } else { raw.wrapping_shl((32 - n) * 8) };
                self.insert_word(DynSolType::FixedBytes(n), B256::from(word));
            }
        }
    }

    /// Attempts to constant-fold a compound expression and seed the resulting value(s).
    ///
    /// This walks recognized casts (`uintN`/`intN`/`bytesN`/`address`), `keccak256` of literal
    /// arguments, `type(T).min`/`max`, and arithmetic/bitwise expressions over numeric literals.
    fn fold_and_seed(&mut self, expr: &ast::Expr<'_>) {
        if let ast::ExprKind::Call(callee, args) = &expr.kind
            && let Some(arg) = single_arg(args)
        {
            match &callee.peel_parens().kind {
                // A top-level `keccak256(<literal>)` seeds a `bytes32` hash; nested, it is folded
                // to its numeric value by `eval` instead.
                ast::ExprKind::Ident(id) if id.as_str() == "keccak256" => {
                    if let Some(bytes) = lit_bytes(arg) {
                        self.insert_word(DynSolType::FixedBytes(32), keccak256(bytes));
                    }
                    return;
                }
                // A top-level `address(...)` cast seeds under `address`; nested, it folds to a
                // 160-bit integer via `cast_num`.
                ast::ExprKind::Type(ty)
                    if matches!(
                        &ty.kind,
                        ast::TypeKind::Elementary(ast::ElementaryType::Address(_))
                    ) =>
                {
                    if let Some(value) = self.eval(arg) {
                        self.insert_word(
                            DynSolType::Address,
                            B256::from(low_bits(value.full_raw(), 160)),
                        );
                    }
                    return;
                }
                _ => {}
            }
        }

        if let Some(value) = self.eval(expr) {
            self.seed_num(value);
        }
    }

    /// Recursively evaluates a constant expression to a numeric value, if possible. Results are
    /// memoized by span and recursion is depth-bounded.
    fn eval(&self, expr: &ast::Expr<'_>) -> Option<Num> {
        self.eval_depth(expr, 0)
    }

    fn eval_depth(&self, expr: &ast::Expr<'_>, depth: usize) -> Option<Num> {
        if depth > MAX_FOLD_DEPTH {
            return None;
        }
        let expr = expr.peel_parens();
        if let Some(cached) = self.eval_cache.borrow().get(&expr.span) {
            return *cached;
        }
        let result = self.eval_kind(expr, depth);
        // Only memoize successful folds, so a depth-limited `None` doesn't poison a subtree that is
        // foldable when later visited as a shallower root.
        if result.is_some() {
            self.eval_cache.borrow_mut().insert(expr.span, result);
        }
        result
    }

    fn eval_kind(&self, expr: &ast::Expr<'_>, depth: usize) -> Option<Num> {
        match &expr.kind {
            ast::ExprKind::Lit(lit, _) => match &lit.kind {
                // Sub-denominations (e.g. `ether`, `days`) are already folded into the value.
                ast::LitKind::Number(n) => Some(Num::untyped(U256::from(*n))),
                _ => None,
            },
            ast::ExprKind::Unary(op, inner) => {
                let value = self.eval_depth(inner, depth + 1)?;
                match op.kind {
                    ast::UnOpKind::Neg => value.neg(),
                    // Bitwise-not complements the raw bits, preserving signedness and width.
                    ast::UnOpKind::BitNot => match value {
                        Num::Int { raw, signed, width } => Some(Num::int(!raw, signed, width)),
                        Num::Bytes { .. } => None,
                    },
                    _ => None,
                }
            }
            ast::ExprKind::Binary(lhs, op, rhs) => {
                let a = self.eval_depth(lhs, depth + 1)?;
                let b = self.eval_depth(rhs, depth + 1)?;
                apply_bin(op.kind, a, b)
            }
            ast::ExprKind::Call(callee, args) => {
                let arg = single_arg(args)?;
                match &callee.peel_parens().kind {
                    ast::ExprKind::Type(ty) => {
                        let ast::TypeKind::Elementary(et) = &ty.kind else { return None };
                        let value = self.eval_depth(arg, depth + 1)?;
                        cast_num(*et, value)
                    }
                    ast::ExprKind::Ident(id) if id.as_str() == "keccak256" => {
                        let bytes = lit_bytes(arg)?;
                        Some(Num::untyped(U256::from_be_bytes(keccak256(bytes).0)))
                    }
                    _ => None,
                }
            }
            // `type(uintN).max`, `type(intN).min`, `type(intN).max`.
            ast::ExprKind::Member(inner, member) => {
                let ast::ExprKind::TypeCall(ty) = &inner.peel_parens().kind else { return None };
                let ast::TypeKind::Elementary(et) = &ty.kind else { return None };
                type_min_max(*et, member.as_str())
            }
            _ => None,
        }
    }
}

impl<'ast> ast::Visit<'ast> for LiteralsCollector {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<()> {
        // Stop early if we've hit the limit
        if self.total_values >= self.max_values {
            return ControlFlow::Break(());
        }

        match &expr.kind {
            // Handle plain literals.
            ast::ExprKind::Lit(lit, _) => match &lit.kind {
                ast::LitKind::Number(n) => self.seed_uint(U256::from(*n)),
                ast::LitKind::Address(addr) => {
                    self.insert_word(DynSolType::Address, addr.into_word())
                }
                ast::LitKind::Str(ast::StrKind::Hex, sym, _) => {
                    self.insert_bytes(Bytes::copy_from_slice(sym.as_byte_str()));
                }
                ast::LitKind::Str(_, sym, _) => {
                    let s = String::from_utf8_lossy(sym.as_byte_str()).into_owned();
                    // For strings, also store the hashed version.
                    self.insert_word(DynSolType::FixedBytes(32), keccak256(s.as_bytes()));
                    // And the right-padded version if it fits.
                    if s.len() <= 32 {
                        self.insert_word(
                            DynSolType::FixedBytes(32),
                            B256::right_padding_from(s.as_bytes()),
                        );
                    }
                    self.insert_string(s);
                }
                ast::LitKind::Bool(..) | ast::LitKind::Rational(..) | ast::LitKind::Err(..) => {
                    // ignore
                }
            },
            // Attempt to constant-fold compound expressions (casts, hashes, arithmetic).
            _ => self.fold_and_seed(expr),
        }

        self.walk_expr(expr)
    }
}

/// A folded constant value. Opportunistic 256-bit heuristic, not a Solidity-correct folder.
#[derive(Clone, Copy, Debug)]
enum Num {
    /// An integer: low-`width` two's-complement bits (right-aligned), signedness, and the bit width
    /// it was cast to (`None` = untyped literal, treated as 256-bit).
    Int { raw: U256, signed: bool, width: Option<usize> },
    /// A `bytesN` value: the `n` significant bytes stored right-aligned.
    Bytes { raw: U256, n: usize },
}

impl Num {
    /// Creates an untyped (256-bit) unsigned integer from a literal.
    const fn untyped(raw: U256) -> Self {
        Self::Int { raw, signed: false, width: None }
    }

    /// Creates an integer, normalizing `raw` to its width's low bits.
    fn int(raw: U256, signed: bool, width: Option<usize>) -> Self {
        let raw = match width {
            Some(bits) => low_bits(raw, bits),
            None => raw,
        };
        Self::Int { raw, signed, width }
    }

    /// Returns the in-width raw bits of the value (right-aligned, not sign-extended).
    const fn as_u256(self) -> U256 {
        match self {
            Self::Int { raw, .. } | Self::Bytes { raw, .. } => raw,
        }
    }

    /// Returns the full 256-bit value, sign-extended from its width in a signed context. Used when
    /// widening through a cast so the sign is preserved.
    fn full_raw(self) -> U256 {
        match self {
            Self::Int { raw, signed: true, width } => sign_extend(raw, width),
            _ => self.as_u256(),
        }
    }

    /// Returns the value as an [`I256`], sign-extended from its width, if it represents a signed
    /// integer (in a signed context, or an unsigned value that fits the positive range).
    fn to_i256(self) -> Option<I256> {
        match self {
            Self::Int { raw, signed: true, width } => Some(I256::from_raw(sign_extend(raw, width))),
            Self::Int { raw, signed: false, .. } => I256::try_from(raw).ok(),
            Self::Bytes { .. } => None,
        }
    }

    /// Returns `true` if the value is in a signed context.
    const fn is_signed(self) -> bool {
        matches!(self, Self::Int { signed: true, .. })
    }

    /// Returns the bit width carried by the value, if any.
    const fn width(self) -> Option<usize> {
        match self {
            Self::Int { width, .. } => width,
            Self::Bytes { .. } => None,
        }
    }

    /// Negates the value, keeping its width and switching to a signed context. Returns `None` if
    /// the result would not fit in [`I256`].
    fn neg(self) -> Option<Self> {
        match self {
            // `-x` fits in `I256` iff `x <= 2**255` (`|I256::MIN|`).
            Self::Int { raw, signed: false, width } => {
                (raw <= I256::MIN.into_raw()).then(|| Self::int(raw.wrapping_neg(), true, width))
            }
            Self::Int { signed: true, width, .. } => {
                self.to_i256()?.checked_neg().map(|i| Self::int(i.into_raw(), true, width))
            }
            Self::Bytes { .. } => None,
        }
    }
}

/// Applies a binary operator to two folded numeric values, carrying the result width so narrow
/// operands produce in-range results.
fn apply_bin(op: ast::BinOpKind, a: Num, b: Num) -> Option<Num> {
    // `bytesN` operands aren't folded as integers.
    if matches!(a, Num::Bytes { .. }) || matches!(b, Num::Bytes { .. }) {
        return None;
    }

    let signed = a.is_signed() || b.is_signed();
    // The result type of a shift is the left operand's type, and of `**` the base's; other
    // operators take the wider operand.
    let width =
        if matches!(op, Shl | Shr | Pow) { a.width() } else { combine_width(a.width(), b.width()) };

    // Bitwise AND/OR/XOR and left-shift operate purely on the (in-width) raw bits and are
    // independent of signedness, so they fold in any context.
    if matches!(op, BitAnd | BitOr | BitXor | Shl) {
        let (x, y) = (a.as_u256(), b.as_u256());
        let raw = match op {
            BitAnd => x & y,
            BitOr => x | y,
            BitXor => x ^ y,
            Shl => shift_amount(y).map_or(U256::ZERO, |s| x.wrapping_shl(s)),
            _ => unreachable!(),
        };
        return Some(Num::int(raw, signed, width));
    }

    // Use signed arithmetic if either operand is in a signed context. Signed `>>` (arithmetic) is
    // not folded here.
    if signed {
        let (x, y) = (a.to_i256()?, b.to_i256()?);
        if op == Pow {
            return signed_pow(x, y, width);
        }
        let r = match op {
            Add => x.wrapping_add(y),
            Sub => x.wrapping_sub(y),
            Mul => x.wrapping_mul(y),
            Div => x.checked_div(y)?,
            Rem => x.checked_rem(y)?,
            _ => return None,
        };
        return Some(Num::int(r.into_raw(), true, width));
    }

    let (x, y) = (a.as_u256(), b.as_u256());
    let r = match op {
        // Wrapping (mod 2**256) matches EVM unchecked arithmetic, so the produced values are ones
        // that can actually occur at runtime.
        Add => x.wrapping_add(y),
        Sub => x.wrapping_sub(y),
        Mul => x.wrapping_mul(y),
        Div => x.checked_div(y)?,
        Rem => x.checked_rem(y)?,
        // `pow` overflowing `U256` means an out-of-range constant, so bail rather than seed a
        // wrapped value. (`2 ** 256 - 1` therefore doesn't fold; use `type(uint256).max`.)
        Pow => x.checked_pow(y)?,
        // Unsigned `>>` is a logical shift.
        Shr => shift_amount(y).map_or(U256::ZERO, |s| x.wrapping_shr(s)),
        // Comparisons and logical operators are not folded into values.
        _ => return None,
    };
    Some(Num::int(r, false, width))
}

/// Folds a signed `base ** exp`, returning `None` on a negative exponent or a magnitude that
/// doesn't fit the signed range (overflow ~ compile error).
fn signed_pow(base: I256, exp: I256, width: Option<usize>) -> Option<Num> {
    if exp.is_negative() {
        return None;
    }
    let magnitude = base.unsigned_abs().checked_pow(exp.into_raw())?;
    let negative = base.is_negative() && exp.into_raw().bit(0);
    if negative {
        // Negative results must fit `[-2**255, 0)`, i.e. magnitude <= |I256::MIN|.
        (magnitude <= I256::MIN.into_raw()).then(|| Num::int(magnitude.wrapping_neg(), true, width))
    } else {
        // Positive results must fit `[0, 2**255)`.
        (magnitude < I256::MIN.into_raw()).then(|| Num::int(magnitude, true, width))
    }
}

/// Combines the widths of two operands: an untyped operand (`None`) inherits the other's width;
/// two explicit widths pick the wider.
fn combine_width(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (None, None) => None,
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(w), None) | (None, Some(w)) => Some(w),
    }
}

/// Folds `type(uintN).max`, `type(intN).min`, and `type(intN).max` (and `type(uintN).min` == 0).
fn type_min_max(ty: ast::ElementaryType, member: &str) -> Option<Num> {
    match (ty, member) {
        (ast::ElementaryType::UInt(size), "max") => {
            let bits = size.bits() as usize;
            Some(Num::int(low_bits(U256::MAX, bits), false, Some(bits)))
        }
        (ast::ElementaryType::UInt(size), "min") => {
            Some(Num::int(U256::ZERO, false, Some(size.bits() as usize)))
        }
        // `intN` max is `2**(N-1) - 1`; min is `-2**(N-1)`, whose two's-complement is just the
        // sign bit set within the width.
        (ast::ElementaryType::Int(size), "max") => {
            let bits = size.bits() as usize;
            Some(Num::int(low_bits(U256::MAX, bits - 1), true, Some(bits)))
        }
        (ast::ElementaryType::Int(size), "min") => {
            let bits = size.bits() as usize;
            Some(Num::int(U256::from(1).wrapping_shl(bits - 1), true, Some(bits)))
        }
        _ => None,
    }
}

/// Returns the shift amount as a `usize`, or `None` if it is `>= 256` (which shifts out all bits).
fn shift_amount(y: U256) -> Option<usize> {
    (y < U256::from(256u64)).then(|| y.as_limbs()[0] as usize)
}

/// Reinterprets a folded value for an elementary type cast used inside a larger expression,
/// applying the target width so truncation and sign-extension are correct (e.g.
/// `int256(uint256(type(uint256).max))` -> `-1`).
fn cast_num(ty: ast::ElementaryType, value: Num) -> Option<Num> {
    match ty {
        ast::ElementaryType::UInt(size) => {
            Some(Num::int(value.full_raw(), false, Some(size.bits() as usize)))
        }
        ast::ElementaryType::Int(size) => {
            Some(Num::int(value.full_raw(), true, Some(size.bits() as usize)))
        }
        ast::ElementaryType::Address(_) => Some(Num::int(value.full_raw(), false, Some(160))),
        ast::ElementaryType::FixedBytes(size) => Some(cast_to_bytes(value, size.bytes() as usize)),
        _ => None,
    }
}

/// Casts a folded value to `bytesN`, keeping the value right-aligned in `Num::Bytes`.
///
/// `bytesM(bytesN)` keeps the leftmost `min(M, N)` bytes (and pads on the right when widening);
/// casting an integer keeps its low `N` bytes.
fn cast_to_bytes(value: Num, n: usize) -> Num {
    let raw = match value {
        Num::Bytes { raw, n: m } if n <= m => raw.wrapping_shr((m - n) * 8),
        Num::Bytes { raw, n: m } => raw.wrapping_shl((n - m) * 8),
        _ => low_bits(value.full_raw(), n * 8),
    };
    Num::Bytes { raw, n }
}

/// Returns the low `bits` of `value`, zeroing everything above.
fn low_bits(value: U256, bits: usize) -> U256 {
    if bits >= 256 { value } else { value & (U256::from(1).wrapping_shl(bits) - U256::from(1)) }
}

/// Sign-extends the low `width` bits of `raw` to a full 256-bit two's-complement value.
fn sign_extend(raw: U256, width: Option<usize>) -> U256 {
    match width {
        Some(bits) if bits < 256 && raw.bit(bits - 1) => raw | U256::MAX.wrapping_shl(bits),
        _ => raw,
    }
}

/// Returns the single positional argument of a call, if it has exactly one.
fn single_arg<'a, 'ast>(args: &'a ast::CallArgs<'ast>) -> Option<&'a ast::Expr<'ast>> {
    let mut exprs = args.exprs();
    (exprs.len() == 1).then(|| exprs.next()).flatten()
}

/// Extracts the raw bytes of a string, unicode, or hex string literal argument, borrowing them
/// directly from the interner to avoid allocating (and re-hashing) on every fold.
fn lit_bytes<'a>(expr: &'a ast::Expr<'_>) -> Option<&'a [u8]> {
    if let ast::ExprKind::Lit(lit, _) = &expr.peel_parens().kind
        && let ast::LitKind::Str(_, sym, _) = &lit.kind
    {
        return Some(sym.as_byte_str());
    }
    None
}

/// Checks if a signed integer value can fit in intN type.
fn can_fit_int(value: I256, bits: usize) -> bool {
    // Calculate the maximum positive value for intN: 2^(N-1) - 1
    let max_val = I256::try_from((U256::from(1) << (bits - 1)) - U256::from(1))
        .expect("max value should fit in I256");
    // Calculate the minimum negative value for intN: -2^(N-1)
    let min_val = -max_val - I256::ONE;

    value >= min_val && value <= max_val
}

/// Checks if an unsigned integer value can fit in uintN type.
fn can_fit_uint(value: U256, bits: usize) -> bool {
    if bits == 256 {
        return true;
    }
    // Calculate the maximum value for uintN: 2^N - 1
    let max_val = (U256::from(1) << bits) - U256::from(1);
    value <= max_val
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use solar::interface::{Session, source_map};

    const SOURCE: &str = r#"
    contract Magic {
        // plain literals
        address constant DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;
        uint64 constant MAGIC_NUMBER = 1122334455;
        int32 constant MAGIC_INT = -777;
        bytes32 constant MAGIC_WORD = "abcd1234";
        bytes constant MAGIC_BYTES = hex"deadbeef";
        string constant MAGIC_STRING = "xyzzy";

        // constant exprs with folding
        uint256 constant NEG_FOLDING = uint(-2);
        uint256 constant BIN_FOLDING = 2 * 2 ether;
        bytes32 constant IMPLEMENTATION_SLOT = bytes32(uint256(keccak256('eip1967.proxy.implementation')) - 1);
    }"#;

    #[test]
    fn test_literals_collector_coverage() {
        let map = process_source_literals(SOURCE);

        // Expected values from the SOURCE contract
        let addr = address!("0x6B175474E89094C44Da98b954EedeAC495271d0F").into_word();
        let num = B256::from(U256::from(1122334455u64));
        let int = B256::from(I256::try_from(-777i32).unwrap().into_raw());
        let word = B256::right_padding_from(b"abcd1234");
        let dyn_bytes = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]);

        assert_word(&map, DynSolType::Address, addr, "Expected DAI in address set");
        assert_word(&map, DynSolType::Uint(64), num, "Expected MAGIC_NUMBER in uint64 set");
        assert_word(&map, DynSolType::Int(32), int, "Expected MAGIC_INT in int32 set");
        assert_word(&map, DynSolType::FixedBytes(32), word, "Expected MAGIC_WORD in bytes32 set");
        assert!(map.strings.contains("xyzzy"), "Expected MAGIC_STRING to be collected");
        assert!(
            map.strings.contains("eip1967.proxy.implementation"),
            "Expected IMPLEMENTATION_SLOT in string set"
        );
        assert!(map.bytes.contains(&dyn_bytes), "Expected MAGIC_BYTES in bytes set");

        // -- folded constant expressions --

        // `uint(-2)` folds to `2**256 - 2`.
        let neg_cast = B256::from(U256::MAX - U256::from(1));
        assert_word(&map, DynSolType::Uint(256), neg_cast, "Expected uint(-2) to be folded");

        // `2 * 2 ether` folds to `4e18`.
        let bin = B256::from(U256::from(4_000_000_000_000_000_000u64));
        assert_word(&map, DynSolType::Uint(64), bin, "Expected `2 * 2 ether` to be folded");

        // `bytes32(uint256(keccak256('eip1967.proxy.implementation')) - 1)` folds to the
        // well-known EIP-1967 implementation slot.
        let slot = B256::from(
            U256::from_be_bytes(keccak256("eip1967.proxy.implementation").0) - U256::from(1),
        );
        assert_word(
            &map,
            DynSolType::FixedBytes(32),
            slot,
            "Expected IMPLEMENTATION_SLOT expression to be folded",
        );
    }

    #[test]
    fn test_literals_collector_size() {
        let literals = process_source_literals(SOURCE);

        // Helper to get count for a type, returns 0 if not present
        let count = |ty: DynSolType| literals.words.get(&ty).map_or(0, |set| set.len());

        assert_eq!(count(DynSolType::Address), 1, "Address literal count mismatch");
        assert_eq!(literals.strings.len(), 3, "String literals count mismatch");
        assert_eq!(literals.bytes.len(), 1, "Byte literals count mismatch");

        // Unsigned integers. Bare literals {1, 2, 777, 1122334455, 2e18} are seeded under every
        // `uintN` that fits, plus the folded values `4e18` (`2 * 2 ether`) and, for `uint256`,
        // `2**256 - 2` (`uint(-2)`), `K` and `K - 1` where `K = keccak256("eip1967...")`.
        assert_eq!(count(DynSolType::Uint(8)), 2, "Uint(8) count mismatch");
        assert_eq!(count(DynSolType::Uint(16)), 3, "Uint(16) count mismatch");
        assert_eq!(count(DynSolType::Uint(32)), 4, "Uint(32) count mismatch");
        assert_eq!(count(DynSolType::Uint(64)), 6, "Uint(64) count mismatch");
        assert_eq!(count(DynSolType::Uint(128)), 6, "Uint(128) count mismatch");
        assert_eq!(count(DynSolType::Uint(256)), 9, "Uint(256) count mismatch");

        // Signed integers - MAGIC_INT (-777) and the folded `-2` appear in multiple sizes; only
        // `-2` fits `int8` (-777 is out of range).
        assert_eq!(count(DynSolType::Int(8)), 1, "Int(8) count mismatch");
        assert_eq!(count(DynSolType::Int(16)), 2, "Int(16) count mismatch");
        assert_eq!(count(DynSolType::Int(32)), 2, "Int(32) count mismatch");
        assert_eq!(count(DynSolType::Int(64)), 2, "Int(64) count mismatch");
        assert_eq!(count(DynSolType::Int(128)), 2, "Int(128) count mismatch");
        assert_eq!(count(DynSolType::Int(256)), 2, "Int(256) count mismatch");

        // FixedBytes(32) includes:
        // - MAGIC_WORD
        // - String literals (hashed and right-padded versions)
        // - The folded EIP-1967 slot `K - 1` (`K` itself dedups with the hashed string literal)
        assert_eq!(count(DynSolType::FixedBytes(32)), 7, "FixedBytes(32) count mismatch");

        // Total count check
        assert_eq!(
            literals.words.values().map(|set| set.len()).sum::<usize>(),
            49,
            "Total word values count mismatch"
        );
    }

    #[test]
    fn test_width_aware_casts() {
        // Casts truncate/sign-extend to the target width, widening respects the source's
        // signedness, and `~`/signed `**` keep the signed context.
        let source = r#"
        contract C {
            uint8 constant A = uint8(-2);                   // 254
            uint8 constant B = uint8(257);                  // 1
            int8 constant D = int8(255);                    // -1
            int256 constant E = int256(1) - 2;              // -1
            int256 constant F = ~int256(0);                 // -1 (not 2**256 - 1)
            int16 constant G = int16(int8(-1));             // -1 (sign-extended)
            int16 constant H = int16(uint8(255));           // 255 (unsigned source)
            uint16 constant I = uint16(int8(-1));           // 65535
            int256 constant J = -2 ** 255;                  // int256 min
            bytes4 constant K = bytes4(uint32(0x12345678)); // left-aligned
        }"#;
        let map = process_source_literals(source);

        let neg_one = B256::from(I256::try_from(-1).unwrap().into_raw());
        assert_word(&map, DynSolType::Uint(8), B256::from(U256::from(254)), "uint8(-2) -> 254");
        assert_word(&map, DynSolType::Uint(8), B256::from(U256::from(1)), "uint8(257) -> 1");
        assert_word(&map, DynSolType::Int(8), neg_one, "int8(255) -> -1");
        assert_word(&map, DynSolType::Int(256), neg_one, "int256(1) - 2 -> -1");
        assert_word(&map, DynSolType::Int(256), neg_one, "~int256(0) -> -1");
        assert_word(&map, DynSolType::Int(16), neg_one, "int16(int8(-1)) -> -1");
        assert_word(
            &map,
            DynSolType::Int(16),
            B256::from(U256::from(255)),
            "int16(uint8(255)) -> 255",
        );
        assert_word(
            &map,
            DynSolType::Uint(16),
            B256::from(U256::from(65535)),
            "uint16(int8(-1)) -> 65535",
        );
        assert_word(
            &map,
            DynSolType::Int(256),
            B256::from(I256::MIN.into_raw()),
            "-2 ** 255 -> int256 min",
        );

        let left_aligned = B256::right_padding_from(&[0x12, 0x34, 0x56, 0x78]);
        assert_word(&map, DynSolType::FixedBytes(4), left_aligned, "bytes4 is left-aligned");
    }

    #[test]
    fn test_width_dependent_ops_stay_in_width() {
        // `~`, shifts, and arithmetic on a narrow operand stay within its width instead of leaking
        // 256-bit results; an untyped literal inherits the typed operand's width.
        let source = r#"
        contract C {
            uint8 constant A = ~uint8(0);                 // 255
            uint8 constant B = uint8(1) << 8;             // 0
            int8 constant D = int8(1) << 7;               // -128
            uint8 constant E = uint8(255) << 256;         // 0 (shift amount >= 256)
            uint8 constant F = uint8(250) + 10;           // 260 -> 4 (typed + untyped literal)
            uint8 constant G = uint8(10) ** uint256(3);   // 1000 -> 232 (result type is base uint8)
            uint8 constant H = uint8(0x80) >> uint256(0); // 128 (result type is left uint8)
        }"#;
        let map = process_source_literals(source);

        assert_word(&map, DynSolType::Uint(8), B256::from(U256::from(255)), "~uint8(0) -> 255");
        assert_word(&map, DynSolType::Uint(8), B256::from(U256::ZERO), "uint8(_) << {8,256} -> 0");
        assert_word(&map, DynSolType::Uint(8), B256::from(U256::from(4)), "uint8(250) + 10 -> 4");
        assert_word(
            &map,
            DynSolType::Uint(8),
            B256::from(U256::from(232)),
            "uint8(10) ** 3 -> 232",
        );
        assert_word(
            &map,
            DynSolType::Uint(8),
            B256::from(U256::from(128)),
            "uint8(0x80) >> 0 -> 128",
        );
        let neg_128 = B256::from(I256::try_from(-128).unwrap().into_raw());
        assert_word(&map, DynSolType::Int(8), neg_128, "int8(1) << 7 -> -128");

        // Untruncated (widened) results must not leak into a larger bucket.
        let leaked =
            |ty, v: u64| map.words.get(&ty).is_some_and(|s| s.contains(&B256::from(U256::from(v))));
        assert!(
            !map.words
                .get(&DynSolType::Uint(256))
                .is_some_and(|s| s.contains(&B256::from(U256::MAX))),
            "~uint8(0) must not seed a uint256 max"
        );
        assert!(
            !leaked(DynSolType::Uint(16), 260),
            "uint8(250) + 10 must not leak 260 into uint16"
        );
        assert!(
            !leaked(DynSolType::Uint(16), 1000),
            "uint8(10) ** 3 must not leak 1000 into uint16"
        );
    }

    #[test]
    fn test_fixed_bytes_folding() {
        // `bytesN` is left-aligned; truncation keeps the leftmost bytes (and must not zero the
        // word), widening pads on the right.
        let source = r#"
        contract C {
            bytes4 constant A = bytes4(uint256(0xdeadbeef12345678)); // low 4 bytes, not zero
            bytes2 constant B = bytes2(bytes4(uint32(0x12345678)));  // keep left -> 0x1234
            bytes4 constant D = bytes4(bytes2(uint16(0x1234)));      // pad right -> 0x12340000
        }"#;
        let map = process_source_literals(source);

        let low4 = B256::right_padding_from(&[0x12, 0x34, 0x56, 0x78]);
        assert_word(&map, DynSolType::FixedBytes(4), low4, "bytes4 must not fold to zero");
        let left2 = B256::right_padding_from(&[0x12, 0x34]);
        assert_word(&map, DynSolType::FixedBytes(2), left2, "bytes2(bytes4(..)) keeps left bytes");
        let padded = B256::right_padding_from(&[0x12, 0x34, 0x00, 0x00]);
        assert_word(&map, DynSolType::FixedBytes(4), padded, "bytes4(bytes2(..)) pads right");
    }

    #[test]
    fn test_type_min_max_folding() {
        let source = r#"
        contract C {
            uint256 constant A = type(uint256).max;
            uint8 constant B = type(uint8).max;        // 255
            int256 constant D = type(int256).min;
            int256 constant E = type(int256).max;
            uint256 constant F = type(uint256).max - 1;
            int8 constant G = type(int8).min;          // -128
            int24 constant H = type(int24).min;        // -2**23
        }"#;
        let map = process_source_literals(source);

        assert_word(&map, DynSolType::Uint(256), B256::from(U256::MAX), "type(uint256).max");
        assert_word(&map, DynSolType::Uint(8), B256::from(U256::from(255)), "type(uint8).max");
        assert_word(
            &map,
            DynSolType::Int(256),
            B256::from(I256::MIN.into_raw()),
            "type(int256).min",
        );
        assert_word(
            &map,
            DynSolType::Int(256),
            B256::from(I256::MAX.into_raw()),
            "type(int256).max",
        );
        let max_minus_one = B256::from(U256::MAX - U256::from(1));
        assert_word(&map, DynSolType::Uint(256), max_minus_one, "type(uint256).max - 1");
        let min8 = B256::from(I256::try_from(-128).unwrap().into_raw());
        assert_word(&map, DynSolType::Int(8), min8, "type(int8).min -> -128");
        let min24 = B256::from(I256::try_from(-(1i64 << 23)).unwrap().into_raw());
        assert_word(&map, DynSolType::Int(24), min24, "type(int24).min -> -2**23");
    }

    #[test]
    fn test_address_cast_seeds_address_type() {
        // A folded `address(...)` cast must be seeded under `address`, not leak into a `uint160`
        // bucket.
        let source = r#"
        contract C {
            address constant A = address(4660);
        }"#;
        let map = process_source_literals(source);

        assert_word(
            &map,
            DynSolType::Address,
            B256::from(low_bits(U256::from(4660), 160)),
            "address(4660) -> address",
        );
        assert_eq!(map.words.get(&DynSolType::Uint(160)), None, "must not seed a uint160 bucket");
    }

    #[test]
    fn test_max_values_is_respected() {
        // Each string literal seeds up to 3 values (hash, padded, and the string itself), so a
        // limit of 2 must stop collection mid-string rather than overrun.
        let source = r#"
        contract C {
            string constant A = "aaa";
            string constant B = "bbb";
            string constant D = "ccc";
        }"#;
        let map = process_source_literals_with_max(source, 2);

        let total = map.words.values().map(|set| set.len()).sum::<usize>()
            + map.strings.len()
            + map.bytes.len();
        assert!(total <= 2, "max_values not respected: collected {total} values");
    }

    // -- TEST HELPERS ---------------------------------------------------------

    fn process_source_literals(source: &str) -> LiteralMaps {
        process_source_literals_with_max(source, usize::MAX)
    }

    fn process_source_literals_with_max(source: &str, max_values: usize) -> LiteralMaps {
        let mut compiler =
            solar::sema::Compiler::new(Session::builder().with_stderr_emitter().build());
        compiler
            .enter_mut(|c| -> std::io::Result<()> {
                let mut pcx = c.parse();
                pcx.set_resolve_imports(false);

                pcx.add_file(
                    c.sess().source_map().new_source_file(source_map::FileName::Stdin, source)?,
                );
                pcx.parse();
                let _ = c.lower_asts();
                Ok(())
            })
            .expect("Failed to compile test source");

        LiteralsCollector::process(&std::sync::Arc::new(compiler), None, max_values)
    }

    fn assert_word(literals: &LiteralMaps, ty: DynSolType, value: B256, msg: &str) {
        assert!(literals.words.get(&ty).is_some_and(|set| set.contains(&value)), "{}", msg);
    }
}
