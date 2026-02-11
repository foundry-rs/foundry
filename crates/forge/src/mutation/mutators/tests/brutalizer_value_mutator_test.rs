//! Integration tests for the BrutalizerValueMutator.
//!
//! Tests that type-cast expressions are XOR-brutalized with a deterministic
//! per-site mask, dirtying unused bits to catch code that doesn't mask properly.

use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};
use std::path::PathBuf;

use crate::mutation::{Session, visitor::MutantVisitor};

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

    assert!(
        !brutalized.is_empty(),
        "address() cast should generate XOR brutalization. All mutations: {:?}",
        mutations.iter().map(|m| format!("{} -> {}", m.original, m.mutation)).collect::<Vec<_>>()
    );
}

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
    assert!(
        brutalized.iter().any(|m| m.mutation.to_string().contains("uint8(")),
        "uint8() cast should be brutalized. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

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
    assert!(
        brutalized.iter().any(|m| m.mutation.to_string().contains("uint128(")),
        "uint128() cast should be brutalized. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

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
    assert!(
        !brutalized.iter().any(|m| m.mutation.to_string().starts_with("uint256(uint256(")),
        "uint256() cast should NOT be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

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
    assert!(
        brutalized.iter().any(|m| m.mutation.to_string().contains("bytes4(")),
        "bytes4() cast should be brutalized. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

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
    assert!(
        brutalized.iter().any(|m| m.mutation.to_string().contains("int16(")),
        "int16() cast should be brutalized. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

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
    let heuristic = mutations
        .iter()
        .filter(|m| {
            matches!(m.mutation, crate::mutation::mutant::MutationType::Brutalized { .. })
                && m.original == "to"
        })
        .count();

    assert_eq!(
        heuristic, 0,
        "Bare identifiers should NOT be brutalized. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

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
        msg_sender_brutalized, 0,
        "msg.sender should NOT be brutalized without explicit cast. Got: {:?}",
        fmt_mutations(&mutations)
    );
}

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

    assert!(
        brutalized.iter().any(|m| m.mutation.to_string().contains("uint128(")),
        "uint128() cast in library should be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
    assert!(
        brutalized.iter().any(|m| m.mutation.to_string().contains("uint64(")),
        "uint64() cast in library should be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

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
    assert!(
        !brutalized.iter().any(|m| m.mutation.to_string().starts_with("bytes32(bytes32(")),
        "bytes32() cast should NOT be brutalized. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

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
    let brutalized = filter_xor_mutations(&mutations);
    assert!(
        brutalized.is_empty(),
        "No type casts means no brutalizations. Got: {:?}",
        fmt_mutations(&brutalized)
    );
}

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

// ── Helpers ──────────────────────────────────────────────────────────────────

fn filter_xor_mutations(
    mutations: &[crate::mutation::mutant::Mutant],
) -> Vec<&crate::mutation::mutant::Mutant> {
    mutations
        .iter()
        .filter(|m| matches!(m.mutation, crate::mutation::mutant::MutationType::Brutalized { .. }))
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
