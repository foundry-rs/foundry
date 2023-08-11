//! Helper functions for suggesting alternative values for a possibly erroneous user input.
use std::cmp::Ordering;

/// Filters multiple strings from a given list of possible values which are similar
/// to the passed in value `v` within a certain confidence by least confidence.
///
/// The jaro winkler similarity boosts candidates that have a common prefix, which is often the case
/// in the event of typos. Thus, in a list of possible values like ["foo", "bar"], the value "fop"
/// will yield `Some("foo")`, whereas "blark" would yield `None`.
pub fn did_you_mean<T, I>(v: &str, candidates: I) -> Vec<String>
where
    T: AsRef<str>,
    I: IntoIterator<Item = T>,
{
    let mut candidates: Vec<(f64, String)> = candidates
        .into_iter()
        .map(|pv| (strsim::jaro_winkler(v, pv.as_ref()), pv.as_ref().to_owned()))
        .filter(|(similarity, _)| *similarity > 0.8)
        .collect();
    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    candidates.into_iter().map(|(_, pv)| pv).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn possible_artifacts_match() {
        let candidates = ["MyContract", "Erc20"];
        assert_eq!(
            did_you_mean("MyCtrac", candidates.iter()).pop(),
            Some("MyContract".to_string())
        );
    }

    #[test]
    fn possible_artifacts_nomatch() {
        let candidates = ["MyContract", "Erc20", "Greeter"];
        assert!(did_you_mean("Vault", candidates.iter()).pop().is_none());
    }
}
