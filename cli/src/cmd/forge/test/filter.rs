use clap::Parser;
use forge::TestFilter;
use regex::Regex;
use std::str::FromStr;

#[derive(Debug, Clone, Parser)]
pub struct Filter {
    /// Only run test functions matching the specified pattern.
    ///
    /// Deprecated: See --match-test
    #[clap(long = "match", short = 'm')]
    pub pattern: Option<regex::Regex>,

    /// Only run test functions matching the specified pattern.
    #[clap(long = "match-test", alias = "mt", conflicts_with = "pattern")]
    pub test_pattern: Option<regex::Regex>,

    /// Only run test functions that do not match the specified pattern.
    #[clap(long = "no-match-test", alias = "nmt", conflicts_with = "pattern")]
    pub test_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in contracts matching the specified pattern.
    #[clap(long = "match-contract", alias = "mc", conflicts_with = "pattern")]
    pub contract_pattern: Option<regex::Regex>,

    /// Only run tests in contracts that do not match the specified pattern.
    #[clap(long = "no-match-contract", alias = "nmc", conflicts_with = "pattern")]
    contract_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in source files matching the specified pattern.
    #[clap(long = "match-path", alias = "mp", conflicts_with = "pattern")]
    pub path_pattern: Option<regex::Regex>,

    /// Only run tests in source files that do not match the specified pattern.
    #[clap(long = "no-match-path", alias = "nmp", conflicts_with = "pattern")]
    pub path_pattern_inverse: Option<regex::Regex>,
}

impl TestFilter for Filter {
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let test_name = test_name.as_ref();
        // Handle the deprecated option match
        if let Some(re) = &self.pattern {
            ok &= re.is_match(test_name);
        }
        if let Some(re) = &self.test_pattern {
            ok &= re.is_match(test_name);
        }
        if let Some(re) = &self.test_pattern_inverse {
            ok &= !re.is_match(test_name);
        }
        ok
    }

    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let contract_name = contract_name.as_ref();
        if let Some(re) = &self.contract_pattern {
            ok &= re.is_match(contract_name);
        }
        if let Some(re) = &self.contract_pattern_inverse {
            ok &= !re.is_match(contract_name);
        }
        ok
    }

    fn matches_path(&self, path: impl AsRef<str>) -> bool {
        let mut ok = true;
        let path = path.as_ref();
        if let Some(re) = &self.path_pattern {
            let re = Regex::from_str(&format!("^{}", re.as_str())).unwrap();
            ok &= re.is_match(path);
        }
        if let Some(re) = &self.path_pattern_inverse {
            let re = Regex::from_str(&format!("^{}", re.as_str())).unwrap();
            ok &= !re.is_match(path);
        }
        ok
    }
}
