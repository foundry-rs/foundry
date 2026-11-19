//! The root-cause matcher: decides which failing tests of different agents prove the same bug.
//!
//! [`root_cause_groups`] is the only entry point, so another matcher can replace this one. A claim
//! with a `@bug <Contract>.<function> <file>:<line>` tag names its target, and two tagged claims
//! agree on the target. Other claims are free text, and the word matcher compares them.

use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

/// Two claims agree when at least 3/10 of their combined words are shared, the threshold of the
/// offline analysis.
const CLAIM_OVERLAP: (usize, usize) = (3, 10);
/// Two tagged claims about one function agree only when their lines are at most this far apart. A
/// function rarely spans more lines, so the window keeps apart functions with one name in one file,
/// such as overloads or functions of two contracts, but not two lines of one function.
const LINE_WINDOW: u64 = 50;
/// Words that do not tell two claims apart.
const STOP_WORDS: [&str; 31] = [
    "the", "a", "an", "of", "to", "is", "in", "on", "and", "or", "must", "be", "it", "that",
    "with", "for", "not", "when", "as", "by", "at", "from", "than", "its", "this", "are", "which",
    "so", "any", "if", "no",
];

/// A `@bug <Contract>.<function> <file>:<line>` tag at the start of a claim, also as the NatSpec
/// tag `@custom:bug`. The contract and an argument list after the function are optional.
static BUG_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@(?:custom:)?bug\s+(?:[A-Za-z_]\w*\.)?([A-Za-z_]\w*)(?:\([^)]*\))?\s+([\w./-]+\.sol):(\d+)")
        .unwrap()
});
/// A Solidity file path.
static SOLIDITY_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w./-]+\.sol").unwrap());
/// A Solidity file path and an optional line.
static SOLIDITY_LOCATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([\w./-]+\.sol)(?::(\d+))?").unwrap());
/// A function name followed by its argument list.
static FUNCTION_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*\(").unwrap());
/// A call that names no function of the code under test, such as a builtin or a type conversion.
static GENERIC_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:require|assert|revert|keccak256|sha256|ecrecover|encode\w*|decode|call|delegatecall|staticcall|address|payable|type|bool|string|bytes\d*|u?int\d*)\($")
        .unwrap()
});
/// A camelCase identifier.
static CAMEL_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b_?[a-z][A-Za-z0-9]*[A-Z][A-Za-z0-9]*\b").unwrap());
/// A lowercase word.
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9_]+").unwrap());

/// The target of a tagged claim: two tagged claims agree when they name the same function in the
/// same file, at lines that are near.
#[derive(Debug, PartialEq)]
struct BugTag {
    /// The path in the tag.
    file: String,
    function: String,
    line: u64,
}

impl BugTag {
    fn new(claim: &str) -> Option<Self> {
        let captures = BUG_TAG.captures(claim)?;
        Some(Self {
            file: captures[2].to_string(),
            function: captures[1].to_string(),
            line: captures[3].parse().ok()?,
        })
    }

    fn agrees(&self, other: &Self) -> bool {
        same_file(&self.file, &other.file)
            && self.function == other.function
            && self.line.abs_diff(other.line) <= LINE_WINDOW
    }
}

/// What two failing tests with free-text claims must share to have one root cause: the file that
/// their claims name, and a function name at near lines or most of the claims' words.
struct RootCause {
    /// The first Solidity path in the claim.
    file: String,
    /// The line after that path, if any.
    line: Option<u64>,
    /// The function calls and camelCase identifiers in the claim, without builtins and type
    /// conversions.
    identifiers: HashSet<String>,
    /// The words of the claim outside Solidity paths, without stop words and words of one or two
    /// letters.
    words: HashSet<String>,
}

impl RootCause {
    fn new(claim: &str) -> Option<Self> {
        let location = SOLIDITY_LOCATION.captures(claim)?;
        let file = location[1].to_string();
        let line = location.get(2).and_then(|line| line.as_str().parse().ok());
        let identifiers = FUNCTION_CALL
            .find_iter(claim)
            .chain(CAMEL_CASE.find_iter(claim))
            .map(|identifier| identifier.as_str())
            .filter(|identifier| !GENERIC_CALL.is_match(identifier))
            .map(String::from)
            .collect();
        let claim = SOLIDITY_FILE.replace_all(claim, " ").to_lowercase();
        let words = WORD
            .find_iter(&claim)
            .map(|word| word.as_str())
            .filter(|word| word.len() > 2 && !STOP_WORDS.contains(word))
            .map(String::from)
            .collect();
        Some(Self { file, line, identifiers, words })
    }

    /// A shared identifier, such as `transfer(` or `totalSupply`, can name a function that many
    /// claims call, so it agrees only when the lines that both claims name, if they do, are near.
    fn agrees(&self, other: &Self) -> bool {
        let shared = self.words.intersection(&other.words).count();
        let all = self.words.union(&other.words).count();
        let near = match (self.line, other.line) {
            (Some(a), Some(b)) => a.abs_diff(b) <= LINE_WINDOW,
            _ => true,
        };
        same_file(&self.file, &other.file)
            && ((near && !self.identifiers.is_disjoint(&other.identifiers))
                || (shared > 0 && shared * CLAIM_OVERLAP.1 >= all * CLAIM_OVERLAP.0))
    }
}

/// Returns whether two Solidity paths of claims can name the same file: one is the other or ends
/// with it. Agents name a file relative to the project, by its file name, or by its path in their
/// copy of the project, so `Vault.sol` and `/tmp/copy/src/Vault.sol` agree with `src/Vault.sol`,
/// but `src/one/Vault.sol` and `src/two/Vault.sol` do not.
fn same_file(a: &str, b: &str) -> bool {
    fn components(path: &str) -> impl Iterator<Item = &str> {
        path.rsplit('/')
            .filter(|component| !matches!(*component, "" | "."))
            .take_while(|component| *component != "..")
    }
    components(a).zip(components(b)).all(|(a, b)| a == b)
}

/// Returns a group number for the claim of each failing test: tests with the same number have one
/// root cause. Two tagged claims agree by their tags. Otherwise the word matcher compares the
/// claims, also when only one of them has a tag. Groups use complete linkage: in claim order, a
/// test joins the first group whose every test agrees with it, else starts a group numbered by its
/// index. A chain of agreeing pairs, such as a bare `Vault.sol` between two `Vault.sol` files or
/// lines that step across the line window, does not link two tests, so it cannot inflate the
/// quorum. A test without a claim, or whose claim names no Solidity file, agrees with no other
/// test.
pub(super) fn root_cause_groups(claims: &[Option<&str>]) -> Vec<usize> {
    let tags = claims.iter().map(|claim| BugTag::new((*claim)?)).collect::<Vec<_>>();
    let causes = claims.iter().map(|claim| RootCause::new((*claim)?)).collect::<Vec<_>>();
    let agree = |i: usize, j: usize| match (&tags[i], &tags[j]) {
        (Some(a), Some(b)) => a.agrees(b),
        _ => matches!((&causes[i], &causes[j]), (Some(a), Some(b)) if a.agrees(b)),
    };
    let mut groups = Vec::<Vec<usize>>::new();
    (0..claims.len())
        .map(|i| match groups.iter_mut().find(|group| group.iter().all(|&j| agree(i, j))) {
            Some(group) => {
                group.push(i);
                group[0]
            }
            None => {
                groups.push(vec![i]);
                i
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause(claim: &str) -> RootCause {
        RootCause::new(claim).unwrap()
    }

    // Two claims agree when they name the same file and share a function name or enough words.
    // The words of the paths do not count, and a claim without a Solidity path has no root cause.
    #[test]
    fn root_causes_agree_on_file_and_function_or_claim_words() {
        let cap = cause("The fee is capped at 100 ether; src/Fee.sol:7 fee() caps at 10 ether.");
        assert!(cap.agrees(&cause("Large amounts pay the maximum: src/Fee.sol:7 fee() is wrong")));
        assert!(
            cap.agrees(&cause("The fee is capped at 100 ether for large amounts (src/Fee.sol)"))
        );
        assert!(!cap.agrees(&cause("The fee is capped at 100 ether; src/Other.sol:7 fee()")));
        assert!(!cap.agrees(&cause("A zero amount is charged; src/Fee.sol:11 charges()")));
        assert!(!cause("Wrong result; src/Fee.sol:7").agrees(&cause("Bad output; src/Fee.sol:7")));
        assert!(RootCause::new("The fee is capped at 100 ether.").is_none());
    }

    // A function that many claims call, or a builtin, does not make two claims about different
    // lines agree. Without a line on both sides, a shared function name still agrees.
    #[test]
    fn root_causes_ignore_generic_calls_and_far_lines() {
        let approve =
            cause("src/Token.sol:40 approve() lets anyone reset the allowance via transfer(");
        let burn = cause("src/Token.sol:200 burn() skips the balance check in transfer(");
        assert!(!approve.agrees(&burn));
        assert!(approve.agrees(&cause("src/Token.sol:44 transfer( drops the allowance check")));
        assert!(approve.agrees(&cause("Allowance resets in src/Token.sol: transfer( is open")));
        assert!(
            !cause("src/Token.sol mint() skips require( on the cap")
                .agrees(&cause("src/Token.sol pause() has no require( on the owner"))
        );
    }

    // Paths agree when one ends with the other, so a file name or a path in an agent's copy of
    // the project names the same file, but paths in different directories do not.
    #[test]
    fn paths_agree_by_their_trailing_components() {
        assert!(same_file("src/Vault.sol", "Vault.sol"));
        assert!(same_file("./src/Vault.sol", "/tmp/forge-properties-1/src/Vault.sol"));
        assert!(!same_file("src/one/Vault.sol", "src/two/Vault.sol"));
        assert!(!same_file("src/Vault.sol", "src/MyVault.sol"));
    }

    // A tag names the function and the source line. The contract and an argument list are
    // optional, and a claim without the tag at its start has none.
    #[test]
    fn bug_tags_parse_the_target() {
        let tag = |file: &str, function: &str, line| BugTag {
            file: file.into(),
            function: function.into(),
            line,
        };
        assert_eq!(
            BugTag::new("@bug Fee.fee src/Fee.sol:7 — the cap is 10 ether"),
            Some(tag("src/Fee.sol", "fee", 7))
        );
        assert_eq!(
            BugTag::new("@bug fee(uint256) contracts/Fee.sol:12 - wrong"),
            Some(tag("contracts/Fee.sol", "fee", 12))
        );
        assert_eq!(
            BugTag::new("@custom:bug Fee.charges Fee.sol:11"),
            Some(tag("Fee.sol", "charges", 11))
        );
        assert_eq!(BugTag::new("The fee is capped; @bug Fee.fee src/Fee.sol:7"), None);
        assert_eq!(BugTag::new("@bug Fee.fee src/Fee.sol — no line"), None);
    }

    // Tagged claims group on the file and the function, whatever their words, and lines far apart
    // or files with one name in different directories split them. Free-text claims, and pairs
    // with only one tag, fall back to the word matcher. A test joins a group only if it agrees
    // with every test in it, so the free-text claim about `fee()` stays apart from the group that
    // holds a tagged claim it does not agree with.
    #[test]
    fn tagged_claims_group_on_file_and_function() {
        let groups = root_cause_groups(&[
            Some("@bug Fee.fee src/Fee.sol:7 — the cap is 10 ether, not 100 ether"),
            Some("@bug FeeLib.fee Fee.sol:8 — large amounts pay too little"),
            Some("@bug Fee.fee src/Other.sol:7 — the cap is 10 ether, not 100 ether"),
            Some("@bug Fee.charges src/Fee.sol:7 — charges() takes a fee on a zero amount"),
            Some("@bug Fee.fee src/Fee.sol:90 — an overload rounds up"),
            Some("A zero amount is charged; src/Fee.sol:11 charges() returns true"),
            Some("The fee is capped at 100 ether; src/Fee.sol:7 fee() caps at 10 ether."),
            None,
            Some("@bug A.f src/one/Vault.sol:2 — returns zero"),
            Some("@bug B.f src/two/Vault.sol:2 — returns one"),
        ]);
        assert_eq!(groups, [0, 0, 2, 3, 4, 3, 6, 7, 8, 9]);
    }

    // Agreeing pairs do not chain: a bare file name agrees with both `Vault.sol` files, and the
    // middle line is near both ends, but each end starts its own group.
    #[test]
    fn groups_need_every_pair_to_agree() {
        let groups = root_cause_groups(&[
            Some("@bug A.f src/one/Vault.sol:2 — returns zero"),
            Some("@bug A.f Vault.sol:2 — returns zero"),
            Some("@bug A.f src/two/Vault.sol:2 — returns zero"),
            Some("@bug A.g src/Pool.sol:7 — wrong"),
            Some("@bug A.g src/Pool.sol:50 — wrong"),
            Some("@bug A.g src/Pool.sol:95 — wrong"),
        ]);
        assert_eq!(groups, [0, 0, 2, 3, 3, 5]);
    }
}
