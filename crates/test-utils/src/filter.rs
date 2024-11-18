use foundry_common::TestFilter;
use regex::Regex;
use std::path::Path;

pub struct Filter {
    test_regex: Regex,
    contract_regex: Regex,
    path_regex: Regex,
    exclude_tests: Option<Regex>,
    exclude_contracts: Option<Regex>,
    exclude_paths: Option<Regex>,
}

impl Filter {
    pub fn new(test_pattern: &str, contract_pattern: &str, path_pattern: &str) -> Self {
        Self {
            test_regex: Regex::new(test_pattern)
                .unwrap_or_else(|_| panic!("Failed to parse test pattern: `{test_pattern}`")),
            contract_regex: Regex::new(contract_pattern).unwrap_or_else(|_| {
                panic!("Failed to parse contract pattern: `{contract_pattern}`")
            }),
            path_regex: Regex::new(path_pattern)
                .unwrap_or_else(|_| panic!("Failed to parse path pattern: `{path_pattern}`")),
            exclude_tests: None,
            exclude_contracts: None,
            exclude_paths: None,
        }
    }

    pub fn contract(contract_pattern: &str) -> Self {
        Self::new(".*", contract_pattern, ".*")
    }

    pub fn path(path_pattern: &str) -> Self {
        Self::new(".*", ".*", path_pattern)
    }

    /// All tests to also exclude
    ///
    /// This is a workaround since regex does not support negative look aheads
    pub fn exclude_tests(mut self, pattern: &str) -> Self {
        self.exclude_tests = Some(Regex::new(pattern).unwrap());
        self
    }

    /// All contracts to also exclude
    ///
    /// This is a workaround since regex does not support negative look aheads
    pub fn exclude_contracts(mut self, pattern: &str) -> Self {
        self.exclude_contracts = Some(Regex::new(pattern).unwrap());
        self
    }

    /// All paths to also exclude
    ///
    /// This is a workaround since regex does not support negative look aheads
    pub fn exclude_paths(mut self, pattern: &str) -> Self {
        self.exclude_paths = Some(Regex::new(pattern).unwrap());
        self
    }

    pub fn matches_all() -> Self {
        Self {
            test_regex: Regex::new(".*").unwrap(),
            contract_regex: Regex::new(".*").unwrap(),
            path_regex: Regex::new(".*").unwrap(),
            exclude_tests: None,
            exclude_contracts: None,
            exclude_paths: None,
        }
    }
}

impl TestFilter for Filter {
    fn matches_test(&self, test_name: &str) -> bool {
        if let Some(exclude) = &self.exclude_tests {
            if exclude.is_match(test_name) {
                return false;
            }
        }
        self.test_regex.is_match(test_name)
    }

    fn matches_contract(&self, contract_name: &str) -> bool {
        if let Some(exclude) = &self.exclude_contracts {
            if exclude.is_match(contract_name) {
                return false;
            }
        }

        self.contract_regex.is_match(contract_name)
    }

    fn matches_path(&self, path: &Path) -> bool {
        let Some(path) = path.to_str() else {
            return false;
        };

        if let Some(exclude) = &self.exclude_paths {
            if exclude.is_match(path) {
                return false;
            }
        }
        self.path_regex.is_match(path)
    }
}
