use solar::{
    ast::{Block, FunctionKind, ItemFunction, Span, StmtKind, Visibility},
    interface::BytePos,
};

use super::{span_seed, splitmix64, transform::Transform};

pub(super) fn assembly_transforms(func: &ItemFunction<'_>) -> Vec<Transform> {
    let Some(body) = &func.body else { return Vec::new() };

    let visibility = func.header.visibility();
    let kind = Some(func.kind);
    if !block_contains_assembly(body) || !is_eligible_function(visibility, kind) {
        return Vec::new();
    }

    let insert_pos = body.span.lo().0 + 1;
    let insert_span = Span::new(BytePos(insert_pos), BytePos(insert_pos));
    vec![
        Transform::Insert {
            offset: insert_pos as usize,
            replacement: generate_memory_brutalization_assembly(insert_span),
        },
        Transform::Insert {
            offset: insert_pos as usize,
            replacement: generate_fmp_misalignment_assembly(insert_span),
        },
    ]
}

const fn is_eligible_function(visibility: Option<Visibility>, kind: Option<FunctionKind>) -> bool {
    if let Some(kind) = kind
        && !matches!(kind, FunctionKind::Function)
    {
        return false;
    }

    matches!(visibility, Some(Visibility::External))
}

fn generate_memory_brutalization_assembly(span: Span) -> String {
    let s = span_seed(span);
    let w0 = splitmix64(s);
    let w1 = splitmix64(s.wrapping_add(1));
    let w2 = splitmix64(s.wrapping_add(2));
    let w3 = splitmix64(s.wrapping_add(3));
    let s0 = splitmix64(s.wrapping_add(4));
    let s1 = splitmix64(s.wrapping_add(5));
    let s2 = splitmix64(s.wrapping_add(6));
    let s3 = splitmix64(s.wrapping_add(7));
    format!(
        " assembly {{ \
        mstore(0x00, 0x{w0:016x}{w1:016x}) \
        mstore(0x20, 0x{w2:016x}{w3:016x}) \
        let _b_p := mload(0x40) \
        mstore(_b_p, 0x{s0:016x}{s1:016x}{s2:016x}{s3:016x}) \
        for {{ let _b_i := 0x20 }} lt(_b_i, 0x400) {{ _b_i := add(_b_i, 0x20) }} {{ \
        mstore(add(_b_p, _b_i), keccak256(add(_b_p, sub(_b_i, 0x20)), 0x20)) \
        }} \
        }} "
    )
}

fn generate_fmp_misalignment_assembly(span: Span) -> String {
    let offset = deterministic_fmp_offset(span);
    format!(" assembly {{ mstore(0x40, add(mload(0x40), {offset})) }} ")
}

fn deterministic_fmp_offset(span: Span) -> u8 {
    ((span_seed(span) % 31) as u8) | 1
}

fn block_contains_assembly(block: &Block<'_>) -> bool {
    block.stmts.iter().any(|stmt| stmt_contains_assembly(&stmt.kind))
}

fn stmt_contains_assembly(kind: &StmtKind<'_>) -> bool {
    match kind {
        StmtKind::Assembly(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => block_contains_assembly(block),
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_contains_assembly(&then_stmt.kind)
                || else_stmt.as_ref().is_some_and(|s| stmt_contains_assembly(&s.kind))
        }
        StmtKind::While(_, body) | StmtKind::DoWhile(body, _) => stmt_contains_assembly(&body.kind),
        StmtKind::For { body, .. } => stmt_contains_assembly(&body.kind),
        StmtKind::Try(try_stmt) => {
            try_stmt.clauses.iter().any(|clause| block_contains_assembly(&clause.block))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::deterministic_fmp_offset;
    use crate::brutalizer::brutalize_source;
    use solar::{ast::Span, interface::BytePos};

    fn brutalize(source: &str) -> String {
        brutalize_source(Path::new("test.sol"), source).unwrap()
    }

    #[test]
    fn memory_injected_for_external_assembly_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() external pure returns (uint256 r) {
        assembly { r := 42 }
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("mstore(0x00,"));
        assert!(result.contains("mstore(0x20,"));
        assert!(result.contains("let _b_p := mload(0x40)"));
        assert!(result.contains("keccak256("));
    }

    #[test]
    fn fmp_injected_for_external_assembly_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() external pure returns (uint256 r) {
        assembly { r := 42 }
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("mstore(0x40, add(mload(0x40),"));
    }

    #[test]
    fn not_injected_for_non_assembly_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() external pure returns (uint256) {
        return 42;
    }
}
"#;
        let result = brutalize(source);
        assert!(!result.contains("mstore(0x00,"));
        assert!(!result.contains("mstore(0x40, add("));
    }

    #[test]
    fn not_injected_for_public_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() public pure returns (uint256 r) {
        assembly { r := 42 }
    }
}
"#;
        let result = brutalize(source);
        assert!(!result.contains("mstore(0x00,"));
    }

    #[test]
    fn not_injected_for_internal_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f() internal pure returns (uint256 r) {
        assembly { r := 42 }
    }
}
"#;
        let result = brutalize(source);
        assert!(!result.contains("mstore(0x00,"));
    }

    #[test]
    fn fmp_offset_is_odd_and_bounded() {
        for i in 0..100u32 {
            let span = Span::new(BytePos(i), BytePos(i + 10));
            let offset = deterministic_fmp_offset(span);
            assert!(offset > 0, "offset must be non-zero");
            assert!(offset < 32, "offset must be < 32");
            assert!(offset % 2 == 1, "offset must be odd");
        }
    }
}
