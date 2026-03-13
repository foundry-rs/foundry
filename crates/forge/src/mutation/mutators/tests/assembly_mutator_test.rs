//! Integration tests for the AssemblyMutator using real-world Solady-inspired patterns.

use std::path::PathBuf;

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};

use crate::mutation::{Session, mutant::MutationType, visitor::MutantVisitor};

/// Solady FixedPointMathLib: `fullMulDivUnchecked` uses mul, sub, div, lt, and, xor.
#[test]
fn test_solady_full_mul_div_unchecked() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function fullMulDivUnchecked(uint256 x, uint256 y, uint256 d)
        internal pure returns (uint256 z)
    {
        assembly {
            z := mul(x, y)
            let mm := mulmod(x, y, not(0))
            let p1 := sub(mm, add(z, lt(mm, z)))
            let t := and(d, sub(0, d))
            d := div(d, t)
            let inv := xor(2, mul(3, d))
            inv := mul(inv, sub(2, mul(d, inv)))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "mul", "add");
    assert_has_opcode_mutation(&yul, "mul", "div");
    assert_has_opcode_mutation(&yul, "sub", "add");
    assert_has_opcode_mutation(&yul, "div", "mul");
    assert_has_opcode_mutation(&yul, "lt", "gt");
    assert_has_opcode_mutation(&yul, "and", "or");
    assert_has_opcode_mutation(&yul, "xor", "and");
    assert_has_opcode_mutation(&yul, "mulmod", "addmod");
}

/// Solady: `divUp` uses iszero, mod, div, add.
#[test]
fn test_solady_div_up() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function divUp(uint256 x, uint256 d) internal pure returns (uint256 z) {
        assembly {
            if iszero(d) { revert(0, 0) }
            z := add(iszero(iszero(mod(x, d))), div(x, d))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "add", "sub");
    assert_has_opcode_mutation(&yul, "div", "mul");
    assert_has_opcode_mutation(&yul, "mod", "div");
}

/// Solady: `zeroFloorSub` / `saturatingSub` uses gt, sub, mul.
#[test]
fn test_solady_zero_floor_sub() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function zeroFloorSub(uint256 x, uint256 y) internal pure returns (uint256 z) {
        assembly {
            z := mul(gt(x, y), sub(x, y))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "gt", "lt");
    assert_has_opcode_mutation(&yul, "sub", "add");
    assert_has_opcode_mutation(&yul, "mul", "add");
}

/// Solady: `min(uint256)` uses lt, xor, mul.
#[test]
fn test_solady_min_unsigned() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function min(uint256 x, uint256 y) internal pure returns (uint256 z) {
        assembly {
            z := xor(x, mul(xor(x, y), lt(y, x)))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "lt", "gt");
    assert_has_opcode_mutation(&yul, "lt", "eq");
    assert_has_opcode_mutation(&yul, "xor", "and");
}

/// Solady: `max(int256)` uses sgt — tests signed comparison mutations.
#[test]
fn test_solady_max_signed() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function max(int256 x, int256 y) internal pure returns (int256 z) {
        assembly {
            z := xor(x, mul(xor(x, y), sgt(y, x)))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "sgt", "slt");
    assert_has_opcode_mutation(&yul, "sgt", "gt");
}

/// Solady: `dist(int256)` uses sgt, sub, xor, add — signed distance.
#[test]
fn test_solady_dist_signed() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function dist(int256 x, int256 y) internal pure returns (uint256 z) {
        assembly {
            z := add(xor(sub(0, sgt(x, y)), sub(y, x)), sgt(x, y))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "sgt", "slt");
    assert_has_opcode_mutation(&yul, "sub", "add");
    assert_has_opcode_mutation(&yul, "add", "sub");
    assert_has_opcode_mutation(&yul, "xor", "or");
}

/// Solady: `saturatingAdd` uses or, sub, lt, add.
#[test]
fn test_solady_saturating_add() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function saturatingAdd(uint256 x, uint256 y) internal pure returns (uint256 z) {
        assembly {
            z := or(sub(0, lt(add(x, y), x)), add(x, y))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "or", "and");
    assert_has_opcode_mutation(&yul, "lt", "gt");
}

/// Solady: `log256` uses shl, shr, lt, or — shift mutations.
#[test]
fn test_solady_log256_shifts() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function log256(uint256 x) internal pure returns (uint256 r) {
        assembly {
            r := shl(7, lt(0xffffffffffffffffffffffffffffffff, x))
            r := or(r, shl(6, lt(0xffffffffffffffff, shr(r, x))))
            r := or(r, shl(5, lt(0xffffffff, shr(r, x))))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "shl", "shr");
    assert_has_opcode_mutation(&yul, "shr", "shl");
    assert_has_opcode_mutation(&yul, "or", "and");
}

/// Solady: `rawAddMod` / `rawMulMod` — addmod ↔ mulmod swaps.
#[test]
fn test_solady_addmod_mulmod() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function rawAddMod(uint256 x, uint256 y, uint256 d) internal pure returns (uint256 z) {
        assembly {
            z := addmod(x, y, d)
        }
    }

    function rawMulMod(uint256 x, uint256 y, uint256 d) internal pure returns (uint256 z) {
        assembly {
            z := mulmod(x, y, d)
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "addmod", "mulmod");
    assert_has_opcode_mutation(&yul, "mulmod", "addmod");
}

/// Opcodes NOT in the mapping should produce zero mutations.
#[test]
fn test_unmapped_opcodes_not_mutated() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test() external view returns (uint256) {
        assembly {
            let x := mload(0x40)
            mstore(0, caller())
            let h := keccak256(0, 32)
            sstore(h, x)
        }
    }
}
"#;

    let yul = yul_mutations(source);
    let bad_opcodes = ["mload", "mstore", "caller", "keccak256", "sstore"];
    for opcode in bad_opcodes {
        assert!(
            !yul.iter().any(|m| m.original_opcode == opcode),
            "'{opcode}' should NOT be mutated"
        );
    }
}

/// Empty assembly block should produce zero Yul mutations.
#[test]
fn test_empty_assembly_block() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test() external pure {
        assembly {}
    }
}
"#;

    let yul = yul_mutations(source);
    assert!(yul.is_empty(), "Empty assembly should produce no mutations");
}

/// Assembly inside a library function body is traversed.
#[test]
fn test_library_assembly_traversal() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library AssemblyLib {
    function addAsm(uint256 a, uint256 b) internal pure returns (uint256 result) {
        assembly {
            result := add(a, b)
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "add", "sub");
    assert_has_opcode_mutation(&yul, "add", "mul");
}

/// Nested calls: only the outermost opcode at each visit should be mutated,
/// inner calls get their own visit.
#[test]
fn test_nested_calls_mutated_independently() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test(uint256 x, uint256 y) external pure returns (uint256) {
        assembly {
            mstore(0, add(mul(x, y), div(x, y)))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "add", "sub");
    assert_has_opcode_mutation(&yul, "mul", "div");
    assert_has_opcode_mutation(&yul, "div", "mul");
}

/// Solidity code without assembly should produce zero Yul mutations.
#[test]
fn test_no_assembly_no_yul_mutations() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract PureSolidity {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }

    function compare(uint256 a, uint256 b) external pure returns (bool) {
        return a < b;
    }
}
"#;

    let yul = yul_mutations(source);
    assert!(yul.is_empty(), "Pure Solidity should produce no Yul mutations");
}

/// Solady: `invMod` uses a for-loop with div, sub, mul, eq, slt, mod.
#[test]
fn test_solady_inv_mod_loop() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function invMod(uint256 a, uint256 n) internal pure returns (uint256 x) {
        assembly {
            let g := n
            let r := mod(a, n)
            for { let y := 1 } 1 {} {
                let q := div(g, r)
                let t := g
                g := r
                r := sub(t, mul(r, q))
                let u := x
                x := y
                y := sub(u, mul(y, q))
                if iszero(r) { break }
            }
            x := mul(eq(g, 1), add(x, mul(slt(x, 0), n)))
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "div", "mul");
    assert_has_opcode_mutation(&yul, "sub", "add");
    assert_has_opcode_mutation(&yul, "mul", "add");
    assert_has_opcode_mutation(&yul, "mod", "div");
    assert_has_opcode_mutation(&yul, "eq", "lt");
    assert_has_opcode_mutation(&yul, "slt", "sgt");
}

/// Solady: `rpow` uses exp — exponentiation mutation.
#[test]
fn test_solady_rpow_exp() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library FixedPointMathLib {
    function rpow(uint256 x, uint256 y, uint256 b) internal pure returns (uint256 z) {
        assembly {
            z := mul(b, iszero(y))
            if x {
                z := xor(b, mul(xor(b, x), and(y, 1)))
                let half := shr(1, b)
                for { y := shr(1, y) } y { y := shr(1, y) } {
                    let xx := mul(x, x)
                    let xxRound := add(xx, half)
                    if or(lt(xxRound, xx), shr(128, x)) {
                        revert(0, 0)
                    }
                    x := shr(1, add(mul(xxRound, xxRound), half))
                }
            }
        }
    }
}
"#;

    let yul = yul_mutations(source);
    assert_has_opcode_mutation(&yul, "shr", "shl");
    assert_has_opcode_mutation(&yul, "xor", "and");
    assert_has_opcode_mutation(&yul, "or", "and");
}

/// Span-based replacement correctly handles the exact opcode token,
/// verified by checking the mutated expression text.
#[test]
fn test_span_replacement_correctness() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test(uint256 a, uint256 b) external pure returns (uint256 r) {
        assembly {
            r := add(a, b)
        }
    }
}
"#;

    let yul = yul_mutations(source);

    let add_to_sub: Vec<_> =
        yul.iter().filter(|m| m.original_opcode == "add" && m.new_opcode == "sub").collect();

    assert_eq!(add_to_sub.len(), 1, "Should have exactly one add->sub mutation");
    assert_eq!(add_to_sub[0].mutated_expr, "sub(a, b)");
}

struct YulMutation {
    original_opcode: String,
    new_opcode: String,
    mutated_expr: String,
}

fn yul_mutations(source: &str) -> Vec<YulMutation> {
    let sess = Session::builder().with_silent_emitter(None).build();

    sess.enter(|| -> solar::interface::Result<Vec<YulMutation>> {
        let arena = Arena::new();
        let mut parser = Parser::from_lazy_source_code(
            &sess,
            &arena,
            FileName::Real(PathBuf::from("test.sol")),
            || Ok(source.to_string()),
        )?;

        let ast = parser.parse_file().map_err(|e| e.emit())?;

        let mut visitor = MutantVisitor::default(PathBuf::from("test.sol")).with_source(source);
        let _ = visitor.visit_source_unit(&ast);

        Ok(visitor
            .mutation_to_conduct
            .into_iter()
            .filter_map(|m| match m.mutation {
                MutationType::YulOpcode { original_opcode, new_opcode, mutated_expr } => {
                    Some(YulMutation { original_opcode, new_opcode, mutated_expr })
                }
                _ => None,
            })
            .collect())
    })
    .unwrap_or_default()
}

fn assert_has_opcode_mutation(mutations: &[YulMutation], from: &str, to: &str) {
    assert!(
        mutations.iter().any(|m| m.original_opcode == from && m.new_opcode == to),
        "Expected mutation {from} -> {to}. Got: [{}]",
        mutations
            .iter()
            .map(|m| format!("{} -> {}", m.original_opcode, m.new_opcode))
            .collect::<Vec<_>>()
            .join(", ")
    );
}
