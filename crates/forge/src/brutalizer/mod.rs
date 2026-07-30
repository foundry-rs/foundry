use std::path::{Path, PathBuf};

use foundry_config::Config;
use solar::{
    ast::{
        Span,
        interface::{Session, source_map::FileName},
    },
    parse::Parser,
};

mod assembly;
mod transform;
mod value;
mod visitor;

use transform::apply_transforms;
use visitor::collect_transforms;

pub fn brutalize_source(path: &Path, source: &str) -> eyre::Result<String> {
    let sess = Session::builder().with_silent_emitter(None).build();

    let result = sess.enter(|| -> solar::interface::Result<_> {
        let arena = solar::ast::Arena::new();
        let ast = {
            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::from(path.to_path_buf()),
                || Ok(source.to_string()),
            )?;
            parser.parse_file().map_err(|e| e.emit())?
        };

        Ok(collect_transforms(source, &ast))
    });

    let transforms = match result {
        Ok(t) => t,
        Err(_) => {
            eyre::bail!("failed to parse {}", path.display());
        }
    };

    Ok(apply_transforms(source, transforms))
}

/// Brutalize all .sol source files in a temp project directory.
///
/// Walks the src directory under `temp_dir`, parses each .sol file, applies all
/// brutalizations (value XOR, memory, FMP), and writes the result back in-place.
///
/// Returns the number of files brutalized.
pub fn brutalize_project(config: &Config, temp_dir: &Path) -> eyre::Result<usize> {
    let src_rel = config.src.strip_prefix(&config.root).unwrap_or(&config.src);
    let src_dir = temp_dir.join(src_rel);

    if !src_dir.exists() {
        return Ok(0);
    }

    let src_rel = src_rel.to_path_buf();
    let skipped_dirs = crate::workspace::handled_project_roots(config)?
        .into_iter()
        .filter(|rel| !rel.as_os_str().is_empty() && *rel != src_rel)
        .map(|rel| temp_dir.join(rel))
        .collect::<Vec<_>>();

    brutalize_sol_files_in_dir(&src_dir, &skipped_dirs)
}

fn brutalize_sol_files_in_dir(dir: &Path, skipped_dirs: &[PathBuf]) -> eyre::Result<usize> {
    let mut count = 0;
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if skipped_dirs.contains(&path) {
            continue;
        }
        if path.is_dir() {
            count += brutalize_sol_files_in_dir(&path, skipped_dirs)?;
        } else if path.extension().is_some_and(|ext| ext == "sol") && !is_test_or_script(&path) {
            let source = std::fs::read_to_string(&path)?;
            let brutalized = brutalize_source(&path, &source)?;
            if brutalized != source {
                std::fs::write(&path, brutalized)?;
                count += 1;
            }
        }
    }
    Ok(count)
}

fn is_test_or_script(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".t.sol") || name.ends_with(".s.sol"))
}

/// Applies the splitmix64 finalizer to produce a well-distributed 64-bit hash.
const fn splitmix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;
    x
}

/// Derives a deterministic seed from a span's byte offsets.
fn span_seed(span: Span) -> u64 {
    let lo = span.lo().0 as u64;
    let hi = span.hi().0 as u64;
    splitmix64(lo.wrapping_mul(0x9e3779b97f4a7c15) ^ hi.wrapping_mul(0xff51afd7ed558ccd))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brutalize(source: &str) -> String {
        brutalize_source(Path::new("test.sol"), source).unwrap()
    }

    #[test]
    fn deterministic_output() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(uint160 x) external pure returns (address) {
        return address(x);
    }
}
"#;
        let r1 = brutalize(source);
        let r2 = brutalize(source);
        assert_eq!(r1, r2);
    }

    #[test]
    fn no_change_without_casts_or_assembly() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    uint256 public x;
    function set(uint256 v) external {
        x = v;
    }
}
"#;
        let result = brutalize(source);
        assert_eq!(result, source);
    }

    #[test]
    fn special_functions_not_brutalized() {
        for body in [
            "constructor() { assembly { sstore(0, 1) } }",
            "fallback() external { assembly { sstore(0, 1) } }",
            "receive() external payable { assembly { sstore(0, 1) } }",
        ] {
            let source = format!("pragma solidity ^0.8.0;\ncontract T {{ {body} }}\n");
            let result = brutalize(&source);
            assert!(!result.contains("mstore(0x00,"), "should not inject for: {body}");
        }

        let free_fn = r#"
pragma solidity ^0.8.0;
function freeFunc() pure returns (uint256 r) {
    assembly { r := 42 }
}
"#;
        let result = brutalize(free_fn);
        assert!(!result.contains("mstore(0x00,"));
    }

    #[test]
    fn assembly_in_nested_control_flow() {
        for body in [
            "if (true) { assembly { r := 42 } }",
            "for (uint256 i; i < 1; i++) { assembly { r := 42 } }",
            "unchecked { assembly { r := 42 } }",
        ] {
            let source = format!(
                "pragma solidity ^0.8.0;\ncontract T {{\n\
                 function f() external pure returns (uint256 r) {{ {body} }}\n}}\n"
            );
            let result = brutalize(&source);
            assert!(result.contains("mstore(0x00,"), "should inject for: {body}");
            assert!(
                result.contains("mstore(0x40, add(mload(0x40),"),
                "should inject FMP for: {body}"
            );
        }
    }

    #[test]
    fn cast_and_assembly_in_same_function() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(uint256 x) external pure returns (uint8 r) {
        r = uint8(x);
        assembly { r := add(r, 1) }
    }
}
"#;
        let result = brutalize(source);
        assert!(result.contains("uint8(uint256(uint8(x)) | (uint256(0x"));
        assert!(result.contains("mstore(0x00,"));
        assert!(result.contains("mstore(0x40, add(mload(0x40),"));
    }

    #[test]
    fn visibility_gates_injection_not_casts() {
        let source = r#"
pragma solidity ^0.8.0;
contract T {
    function f(uint256 x) internal pure returns (uint8) {
        return uint8(x);
    }
    function g(uint256 x) public pure returns (uint8) {
        return uint8(x);
    }
}
"#;
        let result = brutalize(source);
        let count = result.matches("uint8(uint256(uint8(x)) | (uint256(0x").count();
        assert_eq!(count, 2, "casts in internal/public should be brutalized");
        assert!(!result.contains("mstore(0x00,"), "no memory injection for non-external");
    }
}
