/// Internal, additional config values the [TestArgs] supports
#[derive(Debug, Clone)]
pub struct RunTestConfig {
    /// whether to include fuzz tests
    pub include_fuzz_tests: bool,
}

impl RunTestConfig {
    /// Convenience function for [RunTestConfigBuilder::default()]
    pub fn builder() -> RunTestConfigBuilder {
        Default::default()
    }
}

impl Default for RunTestConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Clone)]
pub struct RunTestConfigBuilder {
    /// whether to include fuzz tests
    include_fuzz_tests: bool,
}

impl RunTestConfigBuilder {
    pub fn no_fuzz_tests(mut self) -> Self {
        self.include_fuzz_tests = false;
        self
    }

    pub fn build(self) -> RunTestConfig {
        let RunTestConfigBuilder { include_fuzz_tests } = self;
        RunTestConfig { include_fuzz_tests }
    }
}

impl Default for RunTestConfigBuilder {
    fn default() -> Self {
        Self { include_fuzz_tests: true }
    }
}
