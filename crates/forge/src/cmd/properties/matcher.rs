//! The root-cause matcher: decides which failing tests of different agents prove the same bug.
//!
//! [`root_cause_groups`] is the only entry point, so another matcher can replace this one.

use super::Evidence;
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

/// What two failing tests must share to have one root cause: the target file in the evidence, and
/// a function name or most of the claim's words.
struct RootCause {
    /// The file name of the first Solidity path in `code_path`, else in `source`.
    file: String,
    /// The function calls and camelCase identifiers in `code_path` and `claim`.
    identifiers: HashSet<String>,
    /// The words of `claim`, without stop words and words of one or two letters.
    words: HashSet<String>,
}

impl RootCause {
    fn new(evidence: &Evidence) -> Option<Self> {
        let paths = format!("{} {}", evidence.code_path, evidence.source);
        let file = SOLIDITY_FILE.find(&paths)?.as_str().rsplit('/').next()?.to_string();
        let text = format!("{} {}", evidence.code_path, evidence.claim);
        let identifiers = FUNCTION_CALL
            .find_iter(&text)
            .chain(CAMEL_CASE.find_iter(&text))
            .map(|identifier| identifier.as_str().to_string())
            .collect();
        let claim = evidence.claim.to_lowercase();
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

/// Returns a group number for the evidence of each failing test: tests with the same number have
/// one root cause. Groups use single linkage: a chain of agreeing pairs links two tests. A test
/// without evidence, or whose evidence names no Solidity file, agrees with no other test.
pub(super) fn root_cause_groups(evidence: &[Option<&Evidence>]) -> Vec<usize> {
    let causes =
        evidence.iter().map(|evidence| RootCause::new(evidence.as_ref()?)).collect::<Vec<_>>();
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

    fn cause(claim: &str, code_path: &str) -> RootCause {
        RootCause::new(&Evidence {
            claim: claim.to_string(),
            source: "src/Fee.sol:3".to_string(),
            code_path: code_path.to_string(),
        })
        .unwrap()
    }

    #[test]
    fn root_causes_agree_on_file_and_function_or_claim_words() {
        let cap = cause("The fee is capped at 100 ether.", "src/Fee.sol:7 fee() caps at 10 ether.");
        assert!(
            cap.agrees(&cause("Large amounts pay the maximum.", "src/Fee.sol:7 fee() is wrong"))
        );
        assert!(
            cap.agrees(&cause("The fee is capped at 100 ether for large amounts.", "src/Fee.sol"))
        );
        assert!(!cap.agrees(&cause("The fee is capped at 100 ether.", "src/Other.sol:7 fee()")));
        assert!(!cap.agrees(&cause("A zero amount is charged.", "src/Fee.sol:11 charges()")));
    }
}
