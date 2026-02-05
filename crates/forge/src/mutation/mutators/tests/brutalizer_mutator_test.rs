//! Integration tests for the BrutalizerMutator.
//!
//! The brutalizer mutator XORs type-cast expressions with a deterministic
//! per-site mask, inspired by Solady's Brutalizer.sol. It also injects
//! memory brutalization and FMP misalignment assembly at function entry points.

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};
use std::path::PathBuf;

use crate::mutation::{Session, visitor::MutantVisitor};

/// Test that `address(x)` type casts are brutalized with XOR.
#[test]
fn test_brutalize_address_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test(uint256 x) external pure returns (address) {
        return address(uint160(x));
    }
}
"#;

    let mutations = generate_mutations(source);

    let brutalized = filter_xor_mutations(&mutations);

    eprintln!("Address cast mutations:");
    for m in &brutalized {
        eprintln!("  {} -> {}", m.original, m.mutation);
    }

    assert!(
        !brutalized.is_empty(),
        "address() cast should generate XOR brutalization. All mutations: {:?}",
        mutations.iter().map(|m| format!("{} -> {}", m.original, m.mutation)).collect::<Vec<_>>()
    );
}

/// Test that `uint8(x)` type casts are brutalized.
#[test]
fn test_brutalize_uint8_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function narrow(uint256 x) external pure returns (uint8) {
        return uint8(x);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    let has_uint8 = brutalized.iter().any(|m| m.mutation.to_string().contains("uint8("));
    assert!(has_uint8, "uint8() cast should be brutalized. Got: {:?}", fmt_mutations(&mutations));
}

/// Test that `uint128(x)` type casts are brutalized.
#[test]
fn test_brutalize_uint128_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function narrow(uint256 x) external pure returns (uint128) {
        return uint128(x);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    let has_uint128 = brutalized.iter().any(|m| m.mutation.to_string().contains("uint128("));
    assert!(
        has_uint128,
        "uint128() cast should be brutalized. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

/// Test that `uint256(x)` casts are NOT brutalized (no unused bits).
#[test]
fn test_no_brutalize_uint256_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function widen(uint128 x) external pure returns (uint256) {
        return uint256(x);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    let has_uint256_xor =
        brutalized.iter().any(|m| m.mutation.to_string().starts_with("uint256(uint256("));
    assert!(
        !has_uint256_xor,
        "uint256() cast should NOT be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

/// Test that `bytes4(x)` type casts are brutalized.
#[test]
fn test_brutalize_bytes4_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function selector(bytes32 x) external pure returns (bytes4) {
        return bytes4(x);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    let has_bytes4 = brutalized.iter().any(|m| m.mutation.to_string().contains("bytes4("));
    assert!(has_bytes4, "bytes4() cast should be brutalized. Got: {:?}", fmt_mutations(&mutations));
}

/// Test that `int16(x)` signed integer casts are brutalized.
#[test]
fn test_brutalize_int16_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function narrow(int256 x) external pure returns (int16) {
        return int16(x);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    let has_int16 = brutalized.iter().any(|m| m.mutation.to_string().contains("int16("));
    assert!(has_int16, "int16() cast should be brutalized. Got: {:?}", fmt_mutations(&mutations));
}

/// Test that `bool(x)` casts are NOT brutalized (source-level limitation).
#[test]
fn test_no_brutalize_bool_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function toBool(uint256 x) external pure returns (bool) {
        return x > 0;
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    assert!(
        brutalized.is_empty(),
        "bool should NOT be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

/// Test that bare identifiers are NOT brutalized (no heuristic name matching).
#[test]
fn test_no_heuristic_name_matching() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

contract Example {
    IERC20 public token;

    function send(address to, uint256 amount) external {
        token.transfer(to, amount);
    }
}
"#;

    let mutations = generate_mutations(source);

    // 'to' and 'amount' are bare identifiers in the transfer call —
    // they should NOT be brutalized since we don't know their types from the AST
    let heuristic = mutations
        .iter()
        .filter(|m| {
            matches!(m.mutation, crate::mutation::mutant::MutationType::Brutalized { .. })
                && m.original == "to"
        })
        .count();

    assert_eq!(
        heuristic,
        0,
        "Bare identifiers should NOT be brutalized. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

/// Test that `msg.sender` is NOT brutalized (no heuristic matching).
#[test]
fn test_no_brutalize_msg_sender() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

contract Example {
    IERC20 public token;

    function withdraw(uint256 amount) external {
        token.transfer(msg.sender, amount);
    }
}
"#;

    let mutations = generate_mutations(source);

    let msg_sender_brutalized = mutations
        .iter()
        .filter(|m| {
            matches!(m.mutation, crate::mutation::mutant::MutationType::Brutalized { .. })
                && m.original.contains("msg.sender")
        })
        .count();

    assert_eq!(
        msg_sender_brutalized,
        0,
        "msg.sender should NOT be brutalized without explicit cast. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

/// Test that multiple type casts in one function each get their own mask.
#[test]
fn test_different_masks_per_site() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test(uint256 a, uint256 b) external pure returns (uint8, uint8) {
        return (uint8(a), uint8(b));
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    // Each cast site should get a different mask
    let masks: Vec<String> = brutalized
        .iter()
        .filter_map(|m| {
            let s = m.mutation.to_string();
            s.find("(0x").map(|start| {
                let hex_start = start + 1;
                s[hex_start..].split(')').next().unwrap_or("").to_string()
            })
        })
        .collect();

    if masks.len() >= 2 {
        assert_ne!(masks[0], masks[1], "Different cast sites should produce different masks");
    }
}

/// Test that nested type casts generate mutations at each level.
#[test]
fn test_nested_type_casts() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test(uint256 x) external pure returns (address) {
        return address(uint160(x));
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    // Both address(...) and uint160(...) are type casts
    let has_address =
        brutalized.iter().any(|m| m.mutation.to_string().contains("uint160(uint256("));
    let has_uint160 = brutalized.iter().any(|m| {
        let s = m.mutation.to_string();
        s.contains("uint160(") && s.contains("^ uint256(")
    });

    assert!(
        has_address || has_uint160,
        "Nested casts should generate brutalization. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

/// Test that library functions with type casts are brutalized.
#[test]
fn test_brutalize_library_type_casts() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library SafeCast {
    function toUint128(uint256 value) internal pure returns (uint128) {
        require(value <= type(uint128).max, "overflow");
        return uint128(value);
    }

    function toUint64(uint256 value) internal pure returns (uint64) {
        require(value <= type(uint64).max, "overflow");
        return uint64(value);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    let has_128 = brutalized.iter().any(|m| m.mutation.to_string().contains("uint128("));
    let has_64 = brutalized.iter().any(|m| m.mutation.to_string().contains("uint64("));

    assert!(
        has_128,
        "uint128() cast in library should be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
    assert!(
        has_64,
        "uint64() cast in library should be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

/// Test that `bytes32(x)` is NOT brutalized (no unused bits, like uint256).
#[test]
fn test_no_brutalize_bytes32_cast() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function hash(uint256 x) external pure returns (bytes32) {
        return bytes32(x);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    let has_bytes32_xor =
        brutalized.iter().any(|m| m.mutation.to_string().starts_with("bytes32(bytes32("));
    assert!(
        !has_bytes32_xor,
        "bytes32() cast should NOT be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

/// Test brutalization in a real-world ERC20 approve pattern.
#[test]
fn test_brutalize_erc20_pattern() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Token {
    mapping(address => mapping(address => uint256)) public allowance;

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        if (allowed != type(uint256).max) {
            allowance[from][msg.sender] = allowed - amount;
        }
        return true;
    }
}
"#;

    let mutations = generate_mutations(source);

    // No type casts in this code, so no brutalizations should fire
    let brutalized = filter_xor_mutations(&mutations);
    assert!(
        brutalized.is_empty(),
        "No type casts means no brutalizations. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

/// Test that string and bytes casts are NOT brutalized.
#[test]
fn test_no_brutalize_dynamic_types() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Example {
    function test(string memory s) external pure returns (bytes memory) {
        return bytes(s);
    }
}
"#;

    let mutations = generate_mutations(source);
    let brutalized = filter_xor_mutations(&mutations);

    assert!(
        brutalized.is_empty(),
        "Dynamic types (bytes, string) should NOT be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

// ── Memory brutalization & FMP misalignment tests ───────────────────────────

/// Test memory brutalization on assembly that reads scratch space —
/// the classic bug: assuming 0x00/0x20 are zero before writing to them.
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
    assert!(asm.contains("mstore(0x00, not(0))"), "Should dirty scratch space at 0x00");
    assert!(asm.contains("mstore(0x20, not(0))"), "Should dirty scratch space at 0x20");
    assert!(asm.contains("mload(0x40)"), "Should reference free memory pointer");
}

/// Test memory brutalization on assembly that allocates via FMP —
/// catches code that reads from freshly-allocated memory without initializing.
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
            // Bug: reads ptr+0x20 without writing to it first
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

/// Test FMP misalignment on assembly that assumes word-aligned memory pointer.
#[test]
fn test_fmp_misalignment_aligned_assumption() {
    let source = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract AlignedAllocator {
    function encode(address to, uint256 amount) external pure returns (bytes memory) {
        assembly {
            let ptr := mload(0x40)
            // Assumes ptr is 32-byte aligned
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

/// Test that internal functions do NOT get memory/FMP mutations even with assembly.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert!(
        memory.is_empty(),
        "Internal function should NOT get memory brutalization. Got: {:?}",
        fmt_mutations(&memory)
    );
    assert!(
        fmp.is_empty(),
        "Internal function should NOT get FMP misalignment. Got: {:?}",
        fmt_mutations(&fmp)
    );
}

/// Test that private functions do NOT get memory/FMP mutations even with assembly.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert!(memory.is_empty(), "Private function should NOT get memory brutalization");
    assert!(fmp.is_empty(), "Private function should NOT get FMP misalignment");
}

/// Test that constructors do NOT get memory/FMP mutations.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert!(memory.is_empty(), "Constructor should NOT get memory brutalization");
    assert!(fmp.is_empty(), "Constructor should NOT get FMP misalignment");
}

/// Test that interface functions (no body) do NOT get mutations.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert!(memory.is_empty(), "Interface function should NOT get memory brutalization");
    assert!(fmp.is_empty(), "Interface function should NOT get FMP misalignment");
}

/// Test that multiple external functions with assembly each get their own mutations,
/// while public, internal, and functions without assembly are excluded.
/// Public functions are excluded because they can be called internally (JUMP),
/// sharing the caller's memory — brutalizing would overwrite legitimate state.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert_eq!(
        memory.len(),
        2,
        "Should get memory brutalization for foo and bar only. Got: {:?}",
        fmt_mutations(&memory)
    );
    assert_eq!(
        fmp.len(),
        2,
        "Should get FMP misalignment for foo and bar only. Got: {:?}",
        fmt_mutations(&fmp)
    );
}

/// Test that public functions with assembly do NOT get memory/FMP mutations.
/// Public functions can be called internally (JUMP, shared memory) or externally
/// (CALL, fresh memory). We cannot distinguish at the source level, so we exclude
/// them to avoid false positives from corrupting the caller's memory state.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert!(
        memory.is_empty(),
        "Public function should NOT get memory brutalization (may be called internally). Got: {:?}",
        fmt_mutations(&memory)
    );
    assert!(
        fmp.is_empty(),
        "Public function should NOT get FMP misalignment (may be called internally). Got: {:?}",
        fmt_mutations(&fmp)
    );
}

/// Test that modifiers do NOT get memory/FMP mutations.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert!(memory.is_empty(), "Modifier should NOT get memory brutalization");
    assert!(fmp.is_empty(), "Modifier should NOT get FMP misalignment");
}

/// Test that both value brutalization and memory brutalization coexist
/// in a function that has both type casts and assembly.
#[test]
fn test_value_and_memory_brutalization_coexist() {
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
    let xor = filter_xor_mutations(&mutations);
    let memory = filter_memory_mutations(&mutations);
    let fmp = filter_fmp_mutations(&mutations);

    assert!(!xor.is_empty(), "Should have value brutalization (XOR on type casts)");
    assert!(!memory.is_empty(), "Should have memory brutalization");
    assert!(!fmp.is_empty(), "Should have FMP misalignment");
}

/// Test that public/external functions WITHOUT assembly do NOT get memory/FMP mutations.
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
    let fmp = filter_fmp_mutations(&mutations);

    assert!(
        memory.is_empty(),
        "Function without assembly should NOT get memory brutalization. Got: {:?}",
        fmt_mutations(&memory)
    );
    assert!(
        fmp.is_empty(),
        "Function without assembly should NOT get FMP misalignment. Got: {:?}",
        fmt_mutations(&fmp)
    );
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn filter_xor_mutations(
    mutations: &[crate::mutation::mutant::Mutant],
) -> Vec<&crate::mutation::mutant::Mutant> {
    mutations
        .iter()
        .filter(|m| matches!(m.mutation, crate::mutation::mutant::MutationType::Brutalized { .. }))
        .collect()
}

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
