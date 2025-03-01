use regex::{Match, Regex};
use std::sync::LazyLock as Lazy;

/// A regex that matches the import path and identifier of a solidity import
/// statement with the named groups "path", "id".
// Adapted from <https://github.com/nomiclabs/hardhat/blob/cced766c65b25d3d0beb39ef847246ac9618bdd9/packages/hardhat-core/src/internal/solidity/parse.ts#L100>
pub static RE_SOL_IMPORT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"import\s+(?:(?:"(?P<p1>.*)"|'(?P<p2>.*)')(?:\s+as\s+\w+)?|(?:(?:\w+(?:\s+as\s+\w+)?|\*\s+as\s+\w+|\{\s*(?:\w+(?:\s+as\s+\w+)?(?:\s*,\s*)?)+\s*\})\s+from\s+(?:"(?P<p3>.*)"|'(?P<p4>.*)')))\s*;"#).unwrap()
});

/// A regex that matches an alias within an import statement
pub static RE_SOL_IMPORT_ALIAS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?:(?P<target>\w+)|\*|'|")\s+as\s+(?P<alias>\w+)"#).unwrap());

/// A regex that matches the version part of a solidity pragma
/// as follows: `pragma solidity ^0.5.2;` => `^0.5.2`
/// statement with the named group "version".
// Adapted from <https://github.com/nomiclabs/hardhat/blob/cced766c65b25d3d0beb39ef847246ac9618bdd9/packages/hardhat-core/src/internal/solidity/parse.ts#L119>
pub static RE_SOL_PRAGMA_VERSION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"pragma\s+solidity\s+(?P<version>.+?);").unwrap());

/// A regex that matches the SDPX license identifier
/// statement with the named group "license".
pub static RE_SOL_SDPX_LICENSE_IDENTIFIER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"///?\s*SPDX-License-Identifier:\s*(?P<license>.+)").unwrap());

/// A regex used to remove extra lines in flatenned files
pub static RE_THREE_OR_MORE_NEWLINES: Lazy<Regex> = Lazy::new(|| Regex::new("\n{3,}").unwrap());

/// A regex that matches version pragma in a Vyper
pub static RE_VYPER_VERSION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"#(?:pragma version|@version)\s+(?P<version>.+)").unwrap());

/// A regex that matches the contract names in a Solidity file.
pub static RE_CONTRACT_NAMES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:contract|library|abstract\s+contract|interface)\s+([\w$]+)").unwrap()
});

/// Create a regex that matches any library or contract name inside a file
pub fn create_contract_or_lib_name_regex(name: &str) -> Regex {
    Regex::new(&format!(r#"(?:using\s+(?P<n1>{name})\s+|is\s+(?:\w+\s*,\s*)*(?P<n2>{name})(?:\s*,\s*\w+)*|(?:(?P<ignore>(?:function|error|as)\s+|\n[^\n]*(?:"([^"\n]|\\")*|'([^'\n]|\\')*))|\W+)(?P<n3>{name})(?:\.|\(| ))"#)).unwrap()
}

/// Returns all path parts from any solidity import statement in a string,
/// `import "./contracts/Contract.sol";` -> `"./contracts/Contract.sol"`.
///
/// See also <https://docs.soliditylang.org/en/v0.8.9/grammar.html>
pub fn find_import_paths(contract: &str) -> impl Iterator<Item = Match<'_>> {
    RE_SOL_IMPORT.captures_iter(contract).filter_map(|cap| {
        cap.name("p1")
            .or_else(|| cap.name("p2"))
            .or_else(|| cap.name("p3"))
            .or_else(|| cap.name("p4"))
    })
}

/// Returns the solidity version pragma from the given input:
/// `pragma solidity ^0.5.2;` => `^0.5.2`
pub fn find_version_pragma(contract: &str) -> Option<Match<'_>> {
    RE_SOL_PRAGMA_VERSION.captures(contract)?.name("version")
}

/// Given the regex and the target string, find all occurrences of named groups within the string.
///
/// This method returns the tuple of matches `(a, b)` where `a` is the match for the entire regex
/// and `b` is the match for the first named group.
///
/// NOTE: This method will return the match for the first named group, so the order of passed named
/// groups matters.
pub fn capture_outer_and_inner<'a>(
    content: &'a str,
    regex: &regex::Regex,
    names: &[&str],
) -> Vec<(regex::Match<'a>, regex::Match<'a>)> {
    regex
        .captures_iter(content)
        .filter_map(|cap| {
            let cap_match = names.iter().find_map(|name| cap.name(name));
            cap_match.and_then(|m| cap.get(0).map(|outer| (outer.to_owned(), m)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_find_import_paths() {
        let s = r#"//SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.0;
import "hardhat/console.sol";
import "../contract/Contract.sol";
import { T } from "../Test.sol";
import { T } from '../Test2.sol';
"#;
        assert_eq!(
            vec!["hardhat/console.sol", "../contract/Contract.sol", "../Test.sol", "../Test2.sol"],
            find_import_paths(s).map(|m| m.as_str()).collect::<Vec<&str>>()
        );
    }

    #[test]
    fn can_find_version() {
        let s = r"//SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.0;
";
        assert_eq!(Some("^0.8.0"), find_version_pragma(s).map(|s| s.as_str()));
    }

    #[test]
    fn can_parse_curly_bracket_imports() {
        let s =
            r#"import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";"#;
        let imports: Vec<_> = find_import_paths(s).map(|m| m.as_str()).collect();
        assert_eq!(imports, vec!["@openzeppelin/contracts/utils/ReentrancyGuard.sol"])
    }

    #[test]
    fn can_find_single_quote_imports() {
        let content = r"
// SPDX-License-Identifier: MIT
pragma solidity 0.8.6;

import '@openzeppelin/contracts/access/Ownable.sol';
import '@openzeppelin/contracts/utils/Address.sol';

import './../interfaces/IJBDirectory.sol';
import './../libraries/JBTokens.sol';
        ";
        let imports: Vec<_> = find_import_paths(content).map(|m| m.as_str()).collect();

        assert_eq!(
            imports,
            vec![
                "@openzeppelin/contracts/access/Ownable.sol",
                "@openzeppelin/contracts/utils/Address.sol",
                "./../interfaces/IJBDirectory.sol",
                "./../libraries/JBTokens.sol",
            ]
        );
    }
}
