//! Tests to verify that Morpho Blue's MathLib pattern is properly mutated.
//!
//! This tests the issue where MathLib library functions (mulDivDown, mulDivUp, etc.)
//! were not being mutated correctly.

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};
use std::path::PathBuf;

use crate::mutation::{Session, visitor::MutantVisitor};

/// The exact MathLib from Morpho Blue.
/// We must generate mutations for all binary operators in all functions.
const MORPHO_MATHLIB: &str = r#"
// SPDX-License-Identifier: GPL-2.0-or-later
pragma solidity ^0.8.0;

uint256 constant WAD = 1e18;

/// @title MathLib
/// @author Morpho Labs
/// @custom:contact security@morpho.org
/// @notice Library to manage fixed-point arithmetic.
library MathLib {
    /// @dev Returns (`x` * `y`) / `WAD` rounded down.
    function wMulDown(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivDown(x, y, WAD);
    }

    /// @dev Returns (`x` * `WAD`) / `y` rounded down.
    function wDivDown(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivDown(x, WAD, y);
    }

    /// @dev Returns (`x` * `WAD`) / `y` rounded up.
    function wDivUp(uint256 x, uint256 y) internal pure returns (uint256) {
        return mulDivUp(x, WAD, y);
    }

    /// @dev Returns (`x` * `y`) / `d` rounded down.
    function mulDivDown(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y) / d;
    }

    /// @dev Returns (`x` * `y`) / `d` rounded up.
    function mulDivUp(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y + (d - 1)) / d;
    }

    /// @dev Returns the sum of the first three non-zero terms of a Taylor expansion of e^(nx) - 1.
    function wTaylorCompounded(uint256 x, uint256 n) internal pure returns (uint256) {
        uint256 firstTerm = x * n;
        uint256 secondTerm = mulDivDown(firstTerm, firstTerm, 2 * WAD);
        uint256 thirdTerm = mulDivDown(secondTerm, firstTerm, 3 * WAD);
        return firstTerm + secondTerm + thirdTerm;
    }
}
"#;

/// Test that Morpho Blue's MathLib generates the expected mutations.
#[test]
fn test_morpho_mathlib_mutations() {
    let mutations = generate_mutations(MORPHO_MATHLIB);

    // Print all mutations for debugging
    for m in &mutations {
        eprintln!("Mutation: {} -> {}", m.original, m.mutation);
    }

    // Check we have mutations
    assert!(!mutations.is_empty(), "MathLib should generate mutations. Got 0 mutations.");

    // mulDivDown: (x * y) / d should have at least 2 mutations (one for *, one for /)
    let mul_div_down_mutations = mutations
        .iter()
        .filter(|m| {
            let orig = m.original.trim();
            orig.contains("x * y") || orig.contains("(x * y) / d")
        })
        .count();

    assert!(
        mul_div_down_mutations >= 2,
        "mulDivDown should have mutations for x*y and (x*y)/d. Found {} relevant mutations",
        mul_div_down_mutations
    );

    // mulDivUp: (x * y + (d - 1)) / d should have mutations
    let mul_div_up_mutations = mutations
        .iter()
        .filter(|m| {
            let orig = m.original.trim();
            orig.contains("d - 1") || orig.contains("x * y + (d - 1)")
        })
        .count();

    assert!(
        mul_div_up_mutations >= 2,
        "mulDivUp should have mutations for nested expressions. Found {} relevant mutations",
        mul_div_up_mutations
    );

    // wTaylorCompounded: firstTerm + secondTerm + thirdTerm
    let taylor_mutations = mutations
        .iter()
        .filter(|m| {
            let orig = m.original.trim();
            orig.contains("firstTerm + secondTerm") || orig.contains("secondTerm + thirdTerm")
        })
        .count();

    assert!(
        taylor_mutations >= 2,
        "wTaylorCompounded should have mutations for additions. Found {} relevant mutations",
        taylor_mutations
    );
}

/// Test that we generate mutations for arithmetic in mulDivDown specifically.
#[test]
fn test_muldivdown_arithmetic_mutations() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library MathLib {
    function mulDivDown(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y) / d;
    }
}
"#;

    let mutations = generate_mutations(source);

    // We should have mutations for:
    // 1. x * y (inner multiplication)
    // 2. (x * y) / d (outer division)
    let has_mul_mutation = mutations.iter().any(|m| {
        let s = m.mutation.to_string();
        // x * y should be mutated to x + y, x - y, x / y, etc.
        s.contains("x + y") || s.contains("x - y") || s.contains("x / y")
    });

    let has_div_mutation = mutations.iter().any(|m| {
        let s = m.mutation.to_string();
        // (x * y) / d should be mutated to (x * y) + d, (x * y) - d, (x * y) * d, etc.
        s.contains("+ d") || s.contains("- d") || s.contains("* d")
    });

    assert!(
        has_mul_mutation,
        "Should mutate x * y in mulDivDown. Mutations: {:?}",
        mutations.iter().map(|m| format!("{} -> {}", m.original, m.mutation)).collect::<Vec<_>>()
    );

    assert!(
        has_div_mutation,
        "Should mutate / d in mulDivDown. Mutations: {:?}",
        mutations.iter().map(|m| format!("{} -> {}", m.original, m.mutation)).collect::<Vec<_>>()
    );
}

/// Test mulDivUp with its more complex expression.
#[test]
fn test_muldivup_nested_expression_mutations() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library MathLib {
    function mulDivUp(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y + (d - 1)) / d;
    }
}
"#;

    let mutations = generate_mutations(source);

    // We should have mutations for:
    // 1. x * y
    // 2. d - 1
    // 3. (x * y) + (d - 1)
    // 4. (...) / d

    let mutation_strs: Vec<String> =
        mutations.iter().map(|m| format!("{} -> {}", m.original, m.mutation)).collect();

    // Must have at least 4 mutations from the nested expression
    assert!(
        mutations.len() >= 4,
        "mulDivUp should generate at least 4 mutations for nested expressions. Got {}: {:?}",
        mutations.len(),
        mutation_strs
    );
}

/// Test that library using statement in a contract works.
#[test]
fn test_library_using_statement() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library MathLib {
    function mulDivDown(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y) / d;
    }
}

contract Vault {
    using MathLib for uint256;

    function computeShares(uint256 assets, uint256 totalAssets, uint256 totalShares)
        external
        pure
        returns (uint256)
    {
        return assets * totalShares / totalAssets;
    }
}
"#;

    let mutations = generate_mutations(source);

    // Should have mutations from both the library AND the contract
    let library_mutations = mutations
        .iter()
        .filter(|m| m.original.contains("x * y") || m.original.contains("(x * y) / d"))
        .count();

    let contract_mutations = mutations
        .iter()
        .filter(|m| {
            m.original.contains("assets * totalShares") || m.original.contains("/ totalAssets")
        })
        .count();

    assert!(
        library_mutations > 0,
        "Should mutate library functions. Got: {:?}",
        mutations.iter().map(|m| format!("{}", m)).collect::<Vec<_>>()
    );

    assert!(
        contract_mutations > 0,
        "Should mutate contract functions too. Got: {:?}",
        mutations.iter().map(|m| format!("{}", m)).collect::<Vec<_>>()
    );
}

/// Helper function to generate mutations from source code.
fn generate_mutations(source: &str) -> Vec<crate::mutation::mutant::Mutant> {
    let sess = Session::builder().with_silent_emitter(None).build();

    sess.enter(|| -> solar::interface::Result<Vec<crate::mutation::mutant::Mutant>> {
        let arena = Arena::new();
        let mut parser = Parser::from_lazy_source_code(
            &sess,
            &arena,
            FileName::Real(PathBuf::from("MathLib.sol")),
            || Ok(source.to_string()),
        )?;

        let ast = parser.parse_file().map_err(|e| e.emit())?;

        let mut visitor = MutantVisitor::default(PathBuf::from("MathLib.sol")).with_source(source);
        let _ = visitor.visit_source_unit(&ast);

        Ok(visitor.mutation_to_conduct)
    })
    .unwrap_or_default()
}
