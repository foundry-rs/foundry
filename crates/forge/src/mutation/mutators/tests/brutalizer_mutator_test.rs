//! Tests for the BrutalizerMutator.
//!
//! The brutalizer mutator dirties the upper bits of values to test that code
//! properly handles "dirty" inputs - especially relevant in assembly blocks
//! where values are raw 256-bit words.

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};
use std::path::PathBuf;

use crate::mutation::{Session, visitor::MutantVisitor};

/// Test that Yul identifiers in assembly blocks are brutalized.
#[test]
fn test_brutalize_yul_identifier() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function getBalance(address account) external view returns (uint256) {
        assembly {
            let bal := balance(account)
            mstore(0, bal)
            return(0, 32)
        }
    }
}
"#;

    let mutations = generate_mutations(source);

    // Look for brutalized Yul identifiers
    let brutalized_yul: Vec<_> = mutations
        .iter()
        .filter(|m| {
            let s = m.mutation.to_string();
            s.contains("or(") && s.contains("shl(")
        })
        .collect();

    eprintln!("Brutalized Yul mutations:");
    for m in &brutalized_yul {
        eprintln!("  {} -> {}", m.original, m.mutation);
    }

    assert!(
        !brutalized_yul.is_empty(),
        "Should generate brutalized mutations for Yul identifiers in assembly"
    );

    // Should brutalize both 'account' and 'bal'
    let has_account = brutalized_yul.iter().any(|m| m.original == "account");
    let has_bal = brutalized_yul.iter().any(|m| m.original == "bal");

    assert!(has_account, "Should brutalize 'account' identifier");
    assert!(has_bal, "Should brutalize 'bal' identifier");
}

/// Test that assembly code with calldataload is brutalized.
#[test]
fn test_brutalize_calldataload_result() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract LowLevel {
    function readAddress() external pure returns (address) {
        assembly {
            let addr := calldataload(4)
            mstore(0, addr)
            return(0, 32)
        }
    }
}
"#;

    let mutations = generate_mutations(source);

    // The 'addr' identifier should be brutalized
    let addr_mutations: Vec<_> = mutations
        .iter()
        .filter(|m| {
            let s = m.mutation.to_string();
            s.contains("addr") && s.contains("or(")
        })
        .collect();

    assert!(
        !addr_mutations.is_empty(),
        "Should brutalize the 'addr' identifier. Got mutations: {:?}",
        mutations.iter().map(|m| m.mutation.to_string()).collect::<Vec<_>>()
    );
}

/// Test that multiple assumed bit sizes are generated for Yul identifiers.
#[test]
fn test_brutalize_multiple_bit_sizes() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract BitSizes {
    function example(uint256 x) external pure returns (uint256) {
        assembly {
            let result := add(x, 1)
            mstore(0, result)
            return(0, 32)
        }
    }
}
"#;

    let mutations = generate_mutations(source);

    // Should have brutalizations for different bit sizes (160, 128, 64, 8)
    let x_mutations: Vec<_> = mutations
        .iter()
        .filter(|m| m.original == "x" && m.mutation.to_string().contains("or("))
        .collect();

    // Should have 4 mutations for 'x' (one for each bit size)
    assert!(
        x_mutations.len() >= 4,
        "Should generate multiple bit size brutalizations for 'x'. Got {}",
        x_mutations.len()
    );

    // Verify different bit sizes are present
    let has_160 = x_mutations.iter().any(|m| m.mutation.to_string().contains("shl(160"));
    let has_8 = x_mutations.iter().any(|m| m.mutation.to_string().contains("shl(8"));

    assert!(has_160, "Should have 160-bit brutalization (for address)");
    assert!(has_8, "Should have 8-bit brutalization (for uint8)");
}

/// Test that assembly in a library is brutalized.
#[test]
fn test_brutalize_library_assembly() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library MathLib {
    function unsafeDiv(uint256 x, uint256 y) internal pure returns (uint256 result) {
        assembly {
            result := div(x, y)
        }
    }
}
"#;

    let mutations = generate_mutations(source);

    // Should brutalize x and y in the assembly block
    let brutalized: Vec<_> = mutations
        .iter()
        .filter(|m| {
            m.mutation.to_string().contains("or(") && m.mutation.to_string().contains("shl(")
        })
        .collect();

    assert!(
        brutalized.len() >= 8, // 4 bit sizes * 2 identifiers (x, y)
        "Should brutalize assembly identifiers in library. Got {} mutations",
        brutalized.len()
    );
}

/// Test that Yul function parameters are brutalized.
#[test]
fn test_brutalize_yul_function_params() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract YulFunc {
    function example() external pure returns (uint256) {
        assembly {
            function helper(a, b) -> c {
                c := add(a, b)
            }
            let result := helper(1, 2)
            mstore(0, result)
            return(0, 32)
        }
    }
}
"#;

    let mutations = generate_mutations(source);

    // Should brutalize a, b in the helper function
    let has_a =
        mutations.iter().any(|m| m.original == "a" && m.mutation.to_string().contains("or("));
    let has_b =
        mutations.iter().any(|m| m.original == "b" && m.mutation.to_string().contains("or("));

    assert!(has_a || has_b, "Should brutalize Yul function parameters");
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
