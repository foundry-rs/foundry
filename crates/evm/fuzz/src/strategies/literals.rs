use alloy_dyn_abi::DynSolType;
use alloy_primitives::{
    B256, Bytes, I256, U256, keccak256,
    map::{B256IndexSet, HashMap, IndexSet},
};
use foundry_common::Analysis;
use foundry_compilers::ProjectPathsConfig;
use solar::{
    ast::{self, Visit},
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
}

impl<'ast> ast::Visit<'ast> for LiteralsCollector {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<()> {
        // Stop early if we've hit the limit
        if self.total_values >= self.max_values {
            return ControlFlow::Break(());
        }

        // Handle unary negation of number literals
        if let ast::ExprKind::Unary(un_op, inner_expr) = &expr.kind
            && un_op.kind == ast::UnOpKind::Neg
            && let ast::ExprKind::Lit(lit, _) = &inner_expr.kind
            && let ast::LitKind::Number(n) = &lit.kind
        {
            // Compute the negative I256 value
            if let Ok(pos_i256) = I256::try_from(*n) {
                let neg_value = -pos_i256;
                let neg_b256 = B256::from(neg_value.into_raw());

                // Store under all intN sizes that can represent this value
                for bits in [16, 32, 64, 128, 256] {
                    if can_fit_int(neg_value, bits)
                        && self
                            .output
                            .words
                            .entry(DynSolType::Int(bits))
                            .or_default()
                            .insert(neg_b256)
                    {
                        self.total_values += 1;
                    }
                }
            }

            // Continue walking the expression
            return self.walk_expr(expr);
        }

        // Handle literals
        if let ast::ExprKind::Lit(lit, _) = &expr.kind {
            let is_new = match &lit.kind {
                ast::LitKind::Number(n) => {
                    let pos_value = U256::from(*n);
                    let pos_b256 = B256::from(pos_value);

                    // Store under all uintN sizes that can represent this value
                    for bits in [8, 16, 32, 64, 128, 256] {
                        if can_fit_uint(pos_value, bits)
                            && self
                                .output
                                .words
                                .entry(DynSolType::Uint(bits))
                                .or_default()
                                .insert(pos_b256)
                        {
                            self.total_values += 1;
                        }
                    }
                    false // already handled inserts individually
                }
                ast::LitKind::Address(addr) => self
                    .output
                    .words
                    .entry(DynSolType::Address)
                    .or_default()
                    .insert(addr.into_word()),
                ast::LitKind::Str(ast::StrKind::Hex, sym, _) => {
                    self.output.bytes.insert(Bytes::copy_from_slice(sym.as_byte_str()))
                }
                ast::LitKind::Str(_, sym, _) => {
                    let s = String::from_utf8_lossy(sym.as_byte_str()).into_owned();
                    // For strings, also store the hashed version
                    let hash = keccak256(s.as_bytes());
                    if self.output.words.entry(DynSolType::FixedBytes(32)).or_default().insert(hash)
                    {
                        self.total_values += 1;
                    }
                    // And the right-padded version if it fits.
                    if s.len() <= 32 {
                        let padded = B256::right_padding_from(s.as_bytes());
                        if self
                            .output
                            .words
                            .entry(DynSolType::FixedBytes(32))
                            .or_default()
                            .insert(padded)
                        {
                            self.total_values += 1;
                        }
                    }
                    self.output.strings.insert(s)
                }
                ast::LitKind::Bool(..) | ast::LitKind::Rational(..) | ast::LitKind::Err(..) => {
                    false // ignore
                }
            };

            if is_new {
                self.total_values += 1;
            }
        }

        self.walk_expr(expr)
    }
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
    }

    #[test]
    fn test_literals_collector_size() {
        let literals = process_source_literals(SOURCE);

        // Helper to get count for a type, returns 0 if not present
        let count = |ty: DynSolType| literals.words.get(&ty).map_or(0, |set| set.len());

        assert_eq!(count(DynSolType::Address), 1, "Address literal count mismatch");
        assert_eq!(literals.strings.len(), 3, "String literals count mismatch");
        assert_eq!(literals.bytes.len(), 1, "Byte literals count mismatch");

        // Unsigned integers - MAGIC_NUMBER (1122334455) appears in multiple sizes
        assert_eq!(count(DynSolType::Uint(8)), 2, "Uint(8) count mismatch");
        assert_eq!(count(DynSolType::Uint(16)), 3, "Uint(16) count mismatch");
        assert_eq!(count(DynSolType::Uint(32)), 4, "Uint(32) count mismatch");
        assert_eq!(count(DynSolType::Uint(64)), 5, "Uint(64) count mismatch");
        assert_eq!(count(DynSolType::Uint(128)), 5, "Uint(128) count mismatch");
        assert_eq!(count(DynSolType::Uint(256)), 5, "Uint(256) count mismatch");

        // Signed integers - MAGIC_INT (-777) appears in multiple sizes
        assert_eq!(count(DynSolType::Int(16)), 2, "Int(16) count mismatch");
        assert_eq!(count(DynSolType::Int(32)), 2, "Int(32) count mismatch");
        assert_eq!(count(DynSolType::Int(64)), 2, "Int(64) count mismatch");
        assert_eq!(count(DynSolType::Int(128)), 2, "Int(128) count mismatch");
        assert_eq!(count(DynSolType::Int(256)), 2, "Int(256) count mismatch");

        // FixedBytes(32) includes:
        // - MAGIC_WORD
        // - String literals (hashed and right-padded versions)
        assert_eq!(count(DynSolType::FixedBytes(32)), 6, "FixedBytes(32) count mismatch");

        // Total count check
        assert_eq!(
            literals.words.values().map(|set| set.len()).sum::<usize>(),
            41,
            "Total word values count mismatch"
        );
    }

    // -- TEST HELPERS ---------------------------------------------------------

    fn process_source_literals(source: &str) -> LiteralMaps {
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

        LiteralsCollector::process(&std::sync::Arc::new(compiler), None, usize::MAX)
    }

    fn assert_word(literals: &LiteralMaps, ty: DynSolType, value: B256, msg: &str) {
        assert!(literals.words.get(&ty).is_some_and(|set| set.contains(&value)), "{}", msg);
    }
}
