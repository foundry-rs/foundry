//! The root-cause matcher: decides which failing tests of different agents prove the same bug.
//!
//! [`root_cause_groups`] is the only entry point, so another matcher can replace this one.

use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

/// Two claims agree when at least 3/10 of their combined words are shared, the threshold of the
/// offline analysis.
const CLAIM_OVERLAP: (usize, usize) = (3, 10);
/// Words that do not tell two claims apart.
const STOP_WORDS: [&str; 31] = [
    "the", "a", "an", "of", "to", "is", "in", "on", "and", "or", "must", "be", "it", "that",
    "with", "for", "not", "when", "as", "by", "at", "from", "than", "its", "this", "are", "which",
    "so", "any", "if", "no",
];

/// A Solidity file path.
static SOLIDITY_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w./-]+\.sol").unwrap());
/// A function name followed by its argument list.
static FUNCTION_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*\(").unwrap());
/// A camelCase identifier.
static CAMEL_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b_?[a-z][A-Za-z0-9]*[A-Z][A-Za-z0-9]*\b").unwrap());
/// A lowercase word.
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9_]+").unwrap());

/// What two failing tests must share to have one root cause: the file that their claims name, and
/// a function name or most of the claims' words.
struct RootCause {
    /// The file name of the first Solidity path in the claim.
    file: String,
    /// The function calls and camelCase identifiers in the claim.
    identifiers: HashSet<String>,
    /// The words of the claim outside Solidity paths, without stop words and words of one or two
    /// letters.
    words: HashSet<String>,
}

impl RootCause {
    fn new(claim: &str) -> Option<Self> {
        let file = SOLIDITY_FILE.find(claim)?.as_str().rsplit('/').next()?.to_string();
        let identifiers = FUNCTION_CALL
            .find_iter(claim)
            .chain(CAMEL_CASE.find_iter(claim))
            .map(|identifier| identifier.as_str().to_string())
            .collect();
        let claim = SOLIDITY_FILE.replace_all(claim, " ").to_lowercase();
        let words = WORD
            .find_iter(&claim)
            .map(|word| word.as_str())
            .filter(|word| word.len() > 2 && !STOP_WORDS.contains(word))
            .map(String::from)
            .collect();
        Some(Self { file, identifiers, words })
    }

    fn agrees(&self, other: &Self) -> bool {
        let shared = self.words.intersection(&other.words).count();
        let all = self.words.union(&other.words).count();
        self.file == other.file
            && (!self.identifiers.is_disjoint(&other.identifiers)
                || (shared > 0 && shared * CLAIM_OVERLAP.1 >= all * CLAIM_OVERLAP.0))
    }
}

/// Returns a group number for the claim of each failing test: tests with the same number have one
/// root cause. Groups use single linkage: a chain of agreeing pairs links two tests. A test without
/// a claim, or whose claim names no Solidity file, agrees with no other test.
pub(super) fn root_cause_groups(claims: &[Option<&str>]) -> Vec<usize> {
    let causes = claims.iter().map(|claim| RootCause::new((*claim)?)).collect::<Vec<_>>();
    let mut group_of = (0..causes.len()).collect::<Vec<_>>();
    for (i, a) in causes.iter().enumerate() {
        for (j, b) in causes.iter().enumerate().skip(i + 1) {
            if let (Some(a), Some(b)) = (a, b)
                && a.agrees(b)
                && group_of[i] != group_of[j]
            {
                let (from, to) = (group_of[j], group_of[i]);
                group_of.iter_mut().filter(|group| **group == from).for_each(|group| *group = to);
            }
        }
    }
    group_of
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
}
