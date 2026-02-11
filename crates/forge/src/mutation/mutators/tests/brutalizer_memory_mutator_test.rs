//! Integration tests for the BrutalizerMemoryMutator.
//!
//! Tests that memory brutalization is injected at external function entry points
//! with assembly, and excluded for internal/public/private functions.

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};
use std::path::PathBuf;

use crate::mutation::{Session, visitor::MutantVisitor};

#[test]
fn test_memory_brutalization_scratch_space_reader() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract ScratchSpaceReader {
    function hashPair(bytes32 a, bytes32 b) external pure returns (bytes32) {
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
    let memory = filter_memory_mutations(&mutations);

    assert!(
        !memory.is_empty(),
        "Function with scratch space assembly should get memory brutalization. All mutations: {:?}",
        fmt_mutations(&mutations)
    );

    let asm = memory[0].mutation.to_string();
    assert!(asm.contains("mstore(0x00, 0x"), "Should dirty scratch space at 0x00 with random");
    assert!(asm.contains("mstore(0x20, 0x"), "Should dirty scratch space at 0x20 with random");
    assert!(asm.contains("mload(0x40)"), "Should reference free memory pointer");
}

#[test]
fn test_memory_brutalization_fmp_allocator() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract MemoryAllocator {
    function allocateAndReturn(uint256 x) external pure returns (bytes32) {
        assembly {
            let ptr := mload(0x40)
            mstore(0x40, add(ptr, 0x40))
            let val := mload(add(ptr, 0x20))
            mstore(ptr, add(val, x))
            return(ptr, 0x20)
        }
    }
}
"#;

    let mutations = generate_mutations(source);
    let memory = filter_memory_mutations(&mutations);

    assert!(
        !memory.is_empty(),
        "Function with FMP-based allocation should get memory brutalization. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

#[test]
fn test_no_memory_brutalization_internal() {
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
    let memory = filter_memory_mutations(&mutations);
    assert!(
        memory.is_empty(),
        "Internal function should NOT get memory brutalization. Got: {:?}",
        fmt_mutations(&memory)
    );
}

#[test]
fn test_no_memory_brutalization_private() {
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
    let memory = filter_memory_mutations(&mutations);
    assert!(memory.is_empty(), "Private function should NOT get memory brutalization");
}

#[test]
fn test_no_memory_brutalization_constructor() {
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
    let memory = filter_memory_mutations(&mutations);
    assert!(memory.is_empty(), "Constructor should NOT get memory brutalization");
}

#[test]
fn test_no_memory_brutalization_interface() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IExample {
    function doSomething(uint256 x) external returns (uint256);
}
"#;

    let mutations = generate_mutations(source);
    let memory = filter_memory_mutations(&mutations);
    assert!(memory.is_empty(), "Interface function should NOT get memory brutalization");
}

#[test]
fn test_memory_brutalization_multiple_functions() {
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
    let memory = filter_memory_mutations(&mutations);

    assert_eq!(
        memory.len(),
        2,
        "Should get memory brutalization for foo and bar only. Got: {:?}",
        fmt_mutations(&memory)
    );
}

#[test]
fn test_no_memory_brutalization_public() {
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
    let memory = filter_memory_mutations(&mutations);
    assert!(
        memory.is_empty(),
        "Public function should NOT get memory brutalization. Got: {:?}",
        fmt_mutations(&memory)
    );
}

#[test]
fn test_no_memory_brutalization_modifier() {
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
    let memory = filter_memory_mutations(&mutations);
    assert!(memory.is_empty(), "Modifier should NOT get memory brutalization");
}

#[test]
fn test_no_memory_brutalization_without_assembly() {
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
    let memory = filter_memory_mutations(&mutations);
    assert!(
        memory.is_empty(),
        "Function without assembly should NOT get memory brutalization. Got: {:?}",
        fmt_mutations(&memory)
    );
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn filter_memory_mutations(
    mutations: &[crate::mutation::mutant::Mutant],
) -> Vec<&crate::mutation::mutant::Mutant> {
    mutations
        .iter()
        .filter(|m| {
            matches!(m.mutation, crate::mutation::mutant::MutationType::BrutalizeMemory { .. })
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
