//! Integration tests for the BrutalizerFmpMutator.
//!
//! Tests that FMP misalignment is injected at external function entry points
//! with assembly, and excluded for internal/public/private functions.

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};
use std::path::PathBuf;

use crate::mutation::{Session, visitor::MutantVisitor};

#[test]
fn test_fmp_misalignment_aligned_assumption() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract AlignedAllocator {
    function encode(address to, uint256 amount) external pure returns (bytes memory) {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, 0x40)
            mstore(add(ptr, 0x20), to)
            mstore(add(ptr, 0x40), amount)
            mstore(0x40, add(ptr, 0x60))
            return(ptr, 0x60)
        }
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);

    assert!(
        !fmp.is_empty(),
        "Function with alignment-dependent assembly should get FMP misalignment. Got: {:?}",
        fmt_mutations(&mutations)
    );

    let asm = fmp[0].mutation.to_string();
    assert!(asm.contains("mstore(0x40,"), "Should write to FMP slot");
    assert!(asm.contains("add(mload(0x40),"), "Should add offset to current FMP");
}

#[test]
fn test_no_fmp_misalignment_internal() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function _helper(uint256 x) internal pure returns (uint256) {
        assembly {
            mstore(0x00, x)
            return(0x00, 0x20)
        }
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);
    assert!(
        fmp.is_empty(),
        "Internal function should NOT get FMP misalignment. Got: {:?}",
        fmt_mutations(&fmp)
    );
}

#[test]
fn test_no_fmp_misalignment_private() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function _secret(uint256 x) private pure returns (uint256) {
        assembly {
            mstore(0x00, x)
            return(0x00, 0x20)
        }
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);
    assert!(fmp.is_empty(), "Private function should NOT get FMP misalignment");
}

#[test]
fn test_no_fmp_misalignment_constructor() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    uint256 public value;

    constructor(uint256 x) {
        value = x;
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);
    assert!(fmp.is_empty(), "Constructor should NOT get FMP misalignment");
}

#[test]
fn test_no_fmp_misalignment_interface() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IExample {
    function doSomething(uint256 x) external returns (uint256);
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);
    assert!(fmp.is_empty(), "Interface function should NOT get FMP misalignment");
}

#[test]
fn test_fmp_misalignment_multiple_functions() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function foo(uint256 x) external pure returns (uint256) {
        assembly {
            mstore(0x00, x)
            return(0x00, 0x20)
        }
    }

    function bar(uint256 x) external pure returns (uint256) {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, x)
            return(ptr, 0x20)
        }
    }

    function pubFn(uint256 x) public pure returns (uint256) {
        assembly {
            mstore(0x00, x)
            return(0x00, 0x20)
        }
    }

    function _baz(uint256 x) internal pure returns (uint256) {
        assembly {
            mstore(0x00, x)
            return(0x00, 0x20)
        }
    }

    function noAssembly(uint256 x) external pure returns (uint256) {
        return x + 1;
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);

    assert_eq!(
        fmp.len(),
        2,
        "Should get FMP misalignment for foo and bar only. Got: {:?}",
        fmt_mutations(&fmp)
    );
}

#[test]
fn test_no_fmp_misalignment_public() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function hashPair(bytes32 a, bytes32 b) public pure returns (bytes32) {
        assembly {
            mstore(0x00, a)
            mstore(0x20, b)
            let result := keccak256(0x00, 0x40)
            mstore(0x00, result)
            return(0x00, 0x20)
        }
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);
    assert!(
        fmp.is_empty(),
        "Public function should NOT get FMP misalignment. Got: {:?}",
        fmt_mutations(&fmp)
    );
}

#[test]
fn test_no_fmp_misalignment_modifier() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);
    assert!(fmp.is_empty(), "Modifier should NOT get FMP misalignment");
}

#[test]
fn test_no_fmp_misalignment_without_assembly() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}
"#;

    let mutations = generate_mutations(source);
    let fmp = filter_fmp_mutations(&mutations);
    assert!(
        fmp.is_empty(),
        "Function without assembly should NOT get FMP misalignment. Got: {:?}",
        fmt_mutations(&fmp)
    );
}

#[test]
fn test_value_and_fmp_coexist() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test(uint256 x) external pure returns (address) {
        address result = address(uint160(x));
        assembly {
            mstore(0x00, result)
            return(0x00, 0x20)
        }
    }
}
"#;

    let mutations = generate_mutations(source);
    let xor: Vec<_> = mutations
        .iter()
        .filter(|m| matches!(m.mutation, crate::mutation::mutant::MutationType::Brutalized { .. }))
        .collect();
    let fmp = filter_fmp_mutations(&mutations);

    assert!(!xor.is_empty(), "Should have value brutalization (XOR on type casts)");
    assert!(!fmp.is_empty(), "Should have FMP misalignment");
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn filter_fmp_mutations(
    mutations: &[crate::mutation::mutant::Mutant],
) -> Vec<&crate::mutation::mutant::Mutant> {
    mutations
        .iter()
        .filter(|m| {
            matches!(
                m.mutation,
                crate::mutation::mutant::MutationType::MisalignFreeMemoryPointer { .. }
            )
        })
        .collect()
}

fn fmt_mutations(
    mutations: &[impl std::borrow::Borrow<crate::mutation::mutant::Mutant>],
) -> Vec<String> {
    mutations
        .iter()
        .map(|m| {
            let m = m.borrow();
            format!("{} -> {}", m.original, m.mutation)
        })
        .collect()
}

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
