//! Gas-search arithmetic shared by script execution engines.

/// Narrows an estimated gas limit based on execution outcomes.
pub(crate) struct GasSearch {
    gas_used: u64,
    highest: u64,
    lowest: u64,
    last_highest: u64,
    done: bool,
}

impl GasSearch {
    pub(crate) const fn new(gas_used: u64) -> Self {
        Self {
            gas_used,
            highest: gas_used * 3,
            lowest: gas_used,
            last_highest: gas_used * 3,
            done: false,
        }
    }

    pub(crate) const fn next_limit(&self) -> Option<u64> {
        if !self.done && self.highest - self.lowest > 1 {
            Some((self.highest + self.lowest) / 2)
        } else {
            None
        }
    }

    pub(crate) const fn record(&mut self, limit: u64, needs_more_gas: bool) {
        if needs_more_gas {
            self.lowest = limit;
        } else {
            self.highest = limit;
            // Stop when successive successful estimates differ by less than ten percent.
            if (self.last_highest - self.highest) * 10 / self.last_highest < 1 {
                self.gas_used = self.highest;
                self.done = true;
            } else {
                self.last_highest = self.highest;
            }
        }
    }

    pub(crate) const fn gas_used(&self) -> u64 {
        self.gas_used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_probes_keep_existing_ten_percent_stop() {
        let mut search = GasSearch::new(100);
        for expected in [200, 150, 125, 112, 106] {
            assert_eq!(search.next_limit(), Some(expected));
            search.record(expected, false);
        }
        assert_eq!(search.next_limit(), None);
        assert_eq!(search.gas_used(), 106);
    }

    #[test]
    fn unsuccessful_probes_keep_original_estimate() {
        let mut search = GasSearch::new(100);
        while let Some(limit) = search.next_limit() {
            search.record(limit, true);
        }
        assert_eq!(search.gas_used(), 100);
        assert_eq!(GasSearch::new(0).next_limit(), None);
    }
}
