use std::{cmp::Ordering, error::Error, str::FromStr, time::Duration};

use regex::Regex;

use crate::util::sort::natural_cmp;

/// `Duration` wrapper for parsing seconds from the CLI.
#[derive(Clone, Copy)]
pub(crate) struct ParsedSeconds(pub Duration);

impl FromStr for ParsedSeconds {
    type Err = Box<dyn Error + Send + Sync>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Duration::try_from_secs_f64(f64::from_str(s)?)?))
    }
}

/// The primary action to perform.
#[derive(Clone, Copy, Default)]
pub(crate) enum Action {
    /// Run benchmark loops.
    #[default]
    Bench,

    /// Run benchmarked functions once to ensure they run successfully.
    Test,

    /// List benchmarks.
    List,
}

#[allow(dead_code)]
impl Action {
    #[inline]
    pub fn is_bench(&self) -> bool {
        matches!(self, Self::Bench)
    }

    #[inline]
    pub fn is_test(&self) -> bool {
        matches!(self, Self::Test)
    }

    #[inline]
    pub fn is_list(&self) -> bool {
        matches!(self, Self::List)
    }
}

/// Filters which benchmark to run based on name.
pub(crate) enum Filter {
    Regex(Regex),
    Exact(String),
}

impl Filter {
    /// Returns `true` if a string matches this filter.
    pub fn is_match(&self, s: &str) -> bool {
        match self {
            Self::Regex(r) => r.is_match(s),
            Self::Exact(e) => e == s,
        }
    }
}

/// How to treat benchmarks based on whether they're marked as `#[ignore]`.
#[derive(Copy, Clone, Default)]
pub(crate) enum RunIgnored {
    /// Skip ignored.
    #[default]
    No,

    /// `--include-ignored`.
    Yes,

    /// `--ignored`.
    Only,
}

impl RunIgnored {
    pub fn run_ignored(self) -> bool {
        matches!(self, Self::Yes | Self::Only)
    }

    pub fn run_non_ignored(self) -> bool {
        matches!(self, Self::Yes | Self::No)
    }

    pub fn should_run(self, ignored: bool) -> bool {
        if ignored {
            self.run_ignored()
        } else {
            self.run_non_ignored()
        }
    }
}

/// The attribute to sort benchmarks by.
#[derive(Clone, Copy, Default)]
pub(crate) enum SortingAttr {
    /// Sort by kind, then by name and location.
    #[default]
    Kind,

    /// Sort by name, then by location and kind.
    Name,

    /// Sort by location, then by kind and name.
    Location,
}

impl SortingAttr {
    /// Returns an array containing `self` along with other attributes that
    /// should break ties if attributes are equal.
    pub fn with_tie_breakers(self) -> [Self; 3] {
        use SortingAttr::*;

        match self {
            Kind => [self, Name, Location],
            Name => [self, Location, Kind],
            Location => [self, Kind, Name],
        }
    }

    /// Compares benchmark runtime argument names.
    ///
    /// This takes `&&str` to handle `SortingAttr::Location` since the strings
    /// are considered to be within the same `&[&str]`.
    pub fn cmp_bench_arg_names(self, a: &&str, b: &&str) -> Ordering {
        for attr in self.with_tie_breakers() {
            let ordering = match attr {
                SortingAttr::Kind => Ordering::Equal,

                SortingAttr::Name => 'ordering: {
                    // Compare as integers.
                    match (a.parse::<u128>(), a.parse::<u128>()) {
                        (Ok(a_u128), Ok(b_u128)) => break 'ordering a_u128.cmp(&b_u128),

                        (Ok(_), Err(_)) => {
                            if b.parse::<i128>().is_ok() {
                                // a > b, because b is negative.
                                break 'ordering Ordering::Greater;
                            }
                        }

                        (Err(_), Ok(_)) => {
                            if a.parse::<i128>().is_ok() {
                                // a < b, because a is negative.
                                break 'ordering Ordering::Less;
                            }
                        }

                        (Err(_), Err(_)) => {
                            if let (Ok(a_i128), Ok(b_i128)) = (a.parse::<i128>(), a.parse::<i128>())
                            {
                                break 'ordering a_i128.cmp(&b_i128);
                            }
                        }
                    }

                    // Compare as floats.
                    if let (Ok(a), Ok(b)) = (a.parse::<f64>(), b.parse::<f64>()) {
                        if let Some(ordering) = a.partial_cmp(&b) {
                            break 'ordering ordering;
                        }
                    }

                    natural_cmp(a, b)
                }

                SortingAttr::Location => {
                    let a: *const &str = a;
                    let b: *const &str = b;
                    a.cmp(&b)
                }
            };

            if ordering != Ordering::Equal {
                return ordering;
            }
        }

        Ordering::Equal
    }
}
