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
    interface::source_map::FileName,
};
use std::{
    ops::ControlFlow,
    sync::{Arc, OnceLock},
};

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
        for bits in [16, 32, 64, 128, 256] {
            if can_fit_int(value, bits) {
                self.insert_word(DynSolType::Int(bits), word);
            }
        }
    }

    /// Seeds a folded numeric value, choosing signed or unsigned sizes based on its sign.
    fn seed_num(&mut self, value: Num) {
        match value {
            Num::U(u) => self.seed_uint(u),
            Num::I(i) => self.seed_int(i),
        }
    }

    /// Seeds a folded value under the type it is being cast to.
    fn seed_cast(&mut self, ty: ast::ElementaryType, value: Num) {
        match ty {
            // Truncate to the target width, then reinterpret the bits as unsigned (so that, e.g.,
            // `uint(-2)` -> `2**256 - 2` and `uint8(257)` -> `1`). The exact-width bucket is
            // always seeded; smaller sizes are seeded too when the value fits.
            ast::ElementaryType::UInt(size) => {
                let bits = size.bits() as usize;
                let u = cast_uint(value, bits);
                self.insert_word(DynSolType::Uint(bits), B256::from(u));
                self.seed_uint(u);
            }
            // Truncate and sign-extend to the target width (so that, e.g., `int8(255)` -> `-1`).
            ast::ElementaryType::Int(size) => {
                let bits = size.bits() as usize;
                let i = cast_int(value, bits);
                self.insert_word(DynSolType::Int(bits), B256::from(i.into_raw()));
                self.seed_int(i);
            }
            ast::ElementaryType::FixedBytes(size) => {
                let n = size.bytes() as usize;
                // `bytesN` keeps the low `n` bytes but is left-aligned in the word, while integers
                // are right-aligned. Mask first so high bits never silently zero the result.
                let raw = low_bits(value.as_u256(), n * 8);
                let word = if n >= 32 { raw } else { raw.wrapping_shl((32 - n) * 8) };
                self.insert_word(DynSolType::FixedBytes(n), B256::from(word));
            }
            ast::ElementaryType::Address(_) => {
                self.insert_word(DynSolType::Address, B256::from(low_bits(value.as_u256(), 160)));
            }
            _ => {}
        }
    }

    /// Attempts to constant-fold a compound expression and seed the resulting value(s).
    ///
    /// This walks recognized casts (`uintN`/`intN`/`bytesN`/`address`), `keccak256` of literal
    /// arguments, and arithmetic/bitwise expressions over numeric literals.
    fn fold_and_seed(&mut self, expr: &ast::Expr<'_>) {
        match &expr.kind {
            ast::ExprKind::Call(callee, args) => {
                let Some(arg) = single_arg(args) else { return };
                match &callee.peel_parens().kind {
                    // Type cast: `uint(-2)`, `bytes32(...)`, `address(...)`, etc.
                    ast::ExprKind::Type(ty) => {
                        if let ast::TypeKind::Elementary(et) = &ty.kind
                            && let Some(value) = self.eval(arg)
                        {
                            self.seed_cast(*et, value);
                        }
                    }
                    // `keccak256("...")` / `keccak256(hex"...")`.
                    ast::ExprKind::Ident(id) if id.as_str() == "keccak256" => {
                        if let Some(bytes) = lit_bytes(arg) {
                            self.insert_word(DynSolType::FixedBytes(32), keccak256(&bytes));
                        }
                    }
                    _ => {}
                }
            }
            ast::ExprKind::Unary(..) | ast::ExprKind::Binary(..) => {
                if let Some(value) = self.eval(expr) {
                    self.seed_num(value);
                }
            }
            _ => {}
        }
    }

    /// Recursively evaluates a constant expression to a numeric value, if possible.
    fn eval(&self, expr: &ast::Expr<'_>) -> Option<Num> {
        let expr = expr.peel_parens();
        match &expr.kind {
            ast::ExprKind::Lit(lit, _) => match &lit.kind {
                // Sub-denominations (e.g. `ether`, `days`) are already folded into the value.
                ast::LitKind::Number(n) => Some(Num::U(U256::from(*n))),
                _ => None,
            },
            ast::ExprKind::Unary(op, inner) => {
                let value = self.eval(inner)?;
                match op.kind {
                    ast::UnOpKind::Neg => value.neg(),
                    ast::UnOpKind::BitNot => {
                        // Bitwise-not acts on the raw bits but must preserve the signed context
                        // (e.g. `~int256(0)` -> `-1`, `~uint256(0)` -> `2**256 - 1`).
                        let raw = !value.as_u256();
                        Some(if value.is_signed() {
                            Num::I(I256::from_raw(raw))
                        } else {
                            Num::U(raw)
                        })
                    }
                    _ => None,
                }
            }
            ast::ExprKind::Binary(lhs, op, rhs) => {
                let a = self.eval(lhs)?;
                let b = self.eval(rhs)?;
                apply_bin(op.kind, a, b)
            }
            ast::ExprKind::Call(callee, args) => {
                let arg = single_arg(args)?;
                match &callee.peel_parens().kind {
                    ast::ExprKind::Type(ty) => {
                        let ast::TypeKind::Elementary(et) = &ty.kind else { return None };
                        let value = self.eval(arg)?;
                        cast_num(*et, value)
                    }
                    ast::ExprKind::Ident(id) if id.as_str() == "keccak256" => {
                        let bytes = lit_bytes(arg)?;
                        Some(Num::U(U256::from_be_bytes(keccak256(&bytes).0)))
                    }
                    _ => None,
                }
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

/// A folded numeric value. [`Num::I`] carries values in a signed context (from `intN` casts or
/// unary negation) so arithmetic and seeding stay signed; everything else is an unsigned
/// [`Num::U`].
#[derive(Clone, Copy)]
enum Num {
    U(U256),
    I(I256),
}

impl Num {
    /// Returns the raw 256-bit (two's complement) representation of the value.
    const fn as_u256(self) -> U256 {
        match self {
            Self::U(u) => u,
            Self::I(i) => i.into_raw(),
        }
    }

    /// Returns the value as an [`I256`], if it fits.
    fn to_i256(self) -> Option<I256> {
        match self {
            Self::U(u) => I256::try_from(u).ok(),
            Self::I(i) => Some(i),
        }
    }

    /// Returns `true` if the value is in a signed context.
    const fn is_signed(self) -> bool {
        matches!(self, Self::I(_))
    }

    /// Negates the value, if it fits in [`I256`]. The result stays signed.
    fn neg(self) -> Option<Self> {
        self.to_i256()?.checked_neg().map(Self::I)
    }
}

/// Applies a binary operator to two folded numeric values.
fn apply_bin(op: ast::BinOpKind, a: Num, b: Num) -> Option<Num> {
    let signed = a.is_signed() || b.is_signed();

    // Bitwise AND/OR/XOR and left-shift operate purely on the raw bits and are independent of
    // signedness, so they fold in any context (the signed context is just carried through).
    if matches!(op, BitAnd | BitOr | BitXor | Shl) {
        let (x, y) = (a.as_u256(), b.as_u256());
        let raw = match op {
            BitAnd => x & y,
            BitOr => x | y,
            BitXor => x ^ y,
            Shl => shift_amount(y).map_or(U256::ZERO, |s| x.wrapping_shl(s)),
            _ => unreachable!(),
        };
        return Some(if signed { Num::I(I256::from_raw(raw)) } else { Num::U(raw) });
    }

    // Use signed arithmetic if either operand is in a signed context. Signed `>>` (arithmetic) and
    // `**` are intentionally not folded here.
    if signed {
        let (x, y) = (a.to_i256()?, b.to_i256()?);
        let r = match op {
            Add => x.wrapping_add(y),
            Sub => x.wrapping_sub(y),
            Mul => x.wrapping_mul(y),
            Div => x.checked_div(y)?,
            Rem => x.checked_rem(y)?,
            _ => return None,
        };
        return Some(Num::I(r));
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
        // `pow` overflowing almost always means an out-of-range (compile-error) constant, so bail
        // rather than seed a wrapped/garbage value.
        Pow => x.checked_pow(y)?,
        // Unsigned `>>` is a logical shift.
        Shr => shift_amount(y).map_or(U256::ZERO, |s| x.wrapping_shr(s)),
        // Comparisons and logical operators are not folded into values.
        _ => return None,
    };
    Some(Num::U(r))
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
        ast::ElementaryType::UInt(size) => Some(Num::U(cast_uint(value, size.bits() as usize))),
        ast::ElementaryType::Int(size) => Some(Num::I(cast_int(value, size.bits() as usize))),
        ast::ElementaryType::Address(_) => Some(Num::U(low_bits(value.as_u256(), 160))),
        ast::ElementaryType::FixedBytes(_) => Some(Num::U(value.as_u256())),
        _ => None,
    }
}

/// Returns the low `bits` of `value`, zeroing everything above.
fn low_bits(value: U256, bits: usize) -> U256 {
    if bits >= 256 { value } else { value & (U256::from(1).wrapping_shl(bits) - U256::from(1)) }
}

/// Truncates a folded value to an unsigned integer of the given bit width.
fn cast_uint(value: Num, bits: usize) -> U256 {
    low_bits(value.as_u256(), bits)
}

/// Truncates and sign-extends a folded value to a signed integer of the given bit width.
fn cast_int(value: Num, bits: usize) -> I256 {
    let raw = low_bits(value.as_u256(), bits);
    // Sign-extend if the value's top bit (for its width) is set.
    let extended =
        if bits < 256 && raw.bit(bits - 1) { raw | U256::MAX.wrapping_shl(bits) } else { raw };
    I256::from_raw(extended)
}

/// Returns the single positional argument of a call, if it has exactly one.
fn single_arg<'a, 'ast>(args: &'a ast::CallArgs<'ast>) -> Option<&'a ast::Expr<'ast>> {
    let mut exprs = args.exprs();
    (exprs.len() == 1).then(|| exprs.next()).flatten()
}

/// Extracts the raw bytes of a string, unicode, or hex string literal argument.
fn lit_bytes(expr: &ast::Expr<'_>) -> Option<Vec<u8>> {
    if let ast::ExprKind::Lit(lit, _) = &expr.peel_parens().kind
        && let ast::LitKind::Str(_, sym, _) = &lit.kind
    {
        return Some(sym.as_byte_str().to_vec());
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

        // Signed integers - MAGIC_INT (-777) and the folded `-2` appear in multiple sizes
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
            48,
            "Total word values count mismatch"
        );
    }

    #[test]
    fn test_width_aware_casts() {
        let source = r#"
        contract Casts {
            uint8 constant A = uint8(-2);               // 254
            uint8 constant B = uint8(257);              // 1
            int8 constant C = int8(255);                // -1
            int256 constant D = int256(1) - 2;          // -1
            bytes4 constant E = bytes4(uint32(0x12345678)); // left-aligned
        }"#;
        let map = process_source_literals(source);

        // Casts truncate to the target width.
        assert_word(&map, DynSolType::Uint(8), B256::from(U256::from(254)), "uint8(-2) -> 254");
        assert_word(&map, DynSolType::Uint(8), B256::from(U256::from(1)), "uint8(257) -> 1");

        // Signed casts sign-extend, and signed arithmetic stays signed.
        let neg_one = B256::from(I256::try_from(-1).unwrap().into_raw());
        assert_word(&map, DynSolType::Int(8), neg_one, "int8(255) -> -1");
        assert_word(&map, DynSolType::Int(256), neg_one, "int256(1) - 2 -> -1");

        // `bytesN` is left-aligned in the word.
        let left_aligned = B256::right_padding_from(&[0x12, 0x34, 0x56, 0x78]);
        assert_word(&map, DynSolType::FixedBytes(4), left_aligned, "bytes4 is left-aligned");
    }

    #[test]
    fn test_signed_bitwise_not_is_preserved() {
        // `~int256(0)` must fold to a signed `-1`, not an unsigned `2**256 - 1`.
        let source = r#"
        contract C {
            int256 constant A = ~int256(0); // -1
        }"#;
        let map = process_source_literals(source);

        let neg_one = B256::from(I256::try_from(-1).unwrap().into_raw());
        assert_word(&map, DynSolType::Int(256), neg_one, "~int256(0) -> -1");
    }

    #[test]
    fn test_fixed_bytes_truncation_does_not_zero() {
        // Regression: a value with bits above the target `bytesN` width must keep its low bytes
        // (left-aligned) instead of silently folding to zero.
        let source = r#"
        contract C {
            bytes4 constant A = bytes4(uint256(0xdeadbeef12345678));
        }"#;
        let map = process_source_literals(source);

        let expected = B256::right_padding_from(&[0x12, 0x34, 0x56, 0x78]);
        assert_word(&map, DynSolType::FixedBytes(4), expected, "bytes4 must not fold to zero");
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
