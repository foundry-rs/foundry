//! Tests to verify that Solidity library code is properly mutated.
//!
//! Libraries with internal functions are a common pattern (e.g., MathLib, SafeMath).
//! The mutation visitor must traverse into library function bodies just like contract functions.

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};
use std::path::PathBuf;

use crate::mutation::{Session, visitor::MutantVisitor};

/// Test that mutations are generated for code inside a Solidity library.
#[test]
fn test_library_function_mutations() {
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

    // Should have mutations for:
    // - x * y (binary op)
    // - (x * y) / d (binary op)
    assert!(
        !mutations.is_empty(),
        "Library functions should generate mutations, got 0. \
         This indicates the visitor is not traversing into library function bodies."
    );

    // Verify we got binary operator mutations
    let has_mul_mutation = mutations.iter().any(|m| {
        let s = m.mutation.to_string();
        s.contains("+") || s.contains("-") // x * y -> x + y or x - y
    });
    assert!(
        has_mul_mutation,
        "Should have mutations for binary operators in library. Got: {:?}",
        mutations.iter().map(|m| m.mutation.to_string()).collect::<Vec<_>>()
    );
}

/// Test that mutations are generated for multiple functions in a library.
#[test]
fn test_library_multiple_functions() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function sub(uint256 a, uint256 b) internal pure returns (uint256) {
        return a - b;
    }

    function min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }
}
"#;

    let mutations = generate_mutations(source);

    // Should have mutations from all three functions
    // add: a + b
    // sub: a - b
    // min: a < b
    assert!(
        mutations.len() >= 3,
        "Multiple library functions should all generate mutations. Got {} mutations: {:?}",
        mutations.len(),
        mutations.iter().map(|m| format!("{} -> {}", m.original, m.mutation)).collect::<Vec<_>>()
    );
}

/// Test that library with nested expressions generates mutations at all levels.
#[test]
fn test_library_nested_expressions() {
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

    // Nested expression (x * y + (d - 1)) / d should generate mutations for:
    // - x * y
    // - d - 1
    // - (x * y) + (d - 1)
    // - (...) / d
    assert!(
        mutations.len() >= 4,
        "Nested expressions in library should generate multiple mutations. Got {} mutations",
        mutations.len()
    );
}

/// Test that both contract and library in same file are mutated.
#[test]
fn test_contract_and_library_both_mutated() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library Helper {
    function double(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }
}

contract Main {
    function triple(uint256 x) public pure returns (uint256) {
        return x * 3;
    }
}
"#;

    let mutations = generate_mutations(source);

    // Should have mutations from both library and contract
    let mutation_strs: Vec<String> = mutations.iter().map(|m| m.mutation.to_string()).collect();

    // Both x * 2 and x * 3 should be mutated
    assert!(
        mutations.len() >= 2,
        "Both library and contract should be mutated. Got: {:?}",
        mutation_strs
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
            FileName::Real(PathBuf::from("test.sol")),
            || Ok(source.to_string()),
        )?;

        let ast = parser.parse_file().map_err(|e| e.emit())?;

        let mut visitor = MutantVisitor::default(PathBuf::from("test.sol")).with_source(source);
        let _ = visitor.visit_source_unit(&ast);

        Ok(visitor.mutation_to_conduct)
    })
    .unwrap_or_default()
}
