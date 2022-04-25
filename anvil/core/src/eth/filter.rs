use ethers_core::{
    abi::ethereum_types::BloomInput,
    types::{Address, BlockNumber, Bloom, Filter as EthersFilter, Log, ValueOrArray, H256},
};
use serde::{Deserialize, Serialize};

/// topic filter supports nested arrays/items: `[T1,[T2,T3]]` | `[null,[T2,T3]]`
/// Note: we treat `null` and `[]`
pub type Topics = ValueOrArray<Option<ValueOrArray<Option<H256>>>>;

pub type BloomFilter = Vec<Option<Bloom>>;

/// Filter
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Hash)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    /// Integer block number, or "latest" for the last mined block or "pending", "earliest" for not
    /// yet mined transactions.
    pub from_block: Option<BlockNumber>,
    /// Integer block number, or "latest" for the last mined block or "pending", "earliest" for not
    /// yet mined transactions.
    pub to_block: Option<BlockNumber>,
    /// Restricts the logs returned to the single block with the 32-byte hash blockHash.
    /// Using blockHash is equivalent to fromBlock = toBlock = the block number with hash
    /// blockHash. If blockHash is present in the filter criteria, then neither fromBlock nor
    /// toBlock are allowed.
    pub block_hash: Option<H256>,
    /// Contract address or a list of addresses from which logs should originate.
    pub address: Option<ValueOrArray<Address>>,
    /// The provided topics, if any.
    ///
    /// Topics are order-dependent. Each topic can also be an array with “or” options.
    pub topics: Option<Topics>,
}

// === impl Filter ===

impl Filter {
    /// Returns the numeric value of the `toBlock` field
    pub fn get_to_block_number(&self) -> Option<u64> {
        self.to_block.and_then(|block| match block {
            BlockNumber::Number(num) => Some(num.as_u64()),
            _ => None,
        })
    }

    /// Returns the numeric value of the `fromBlock` field
    pub fn get_from_block_number(&self) -> Option<u64> {
        self.from_block.and_then(|block| match block {
            BlockNumber::Number(num) => Some(num.as_u64()),
            _ => None,
        })
    }
}

// this is horrible
impl From<Filter> for EthersFilter {
    fn from(f: Filter) -> Self {
        let Filter { from_block, to_block, block_hash, address, topics } = f;

        let mut filter = EthersFilter::new();
        if let Some(address) = address {
            filter = filter.address(address);
        }
        if let Some(block_hash) = block_hash {
            filter = filter.at_block_hash(block_hash);
        }
        if let Some(from_block) = from_block {
            filter = filter.from_block(from_block);
        }
        if let Some(to_block) = to_block {
            filter = filter.to_block(to_block);
        }
        if let Some(topics) = topics {
            match topics {
                Topics::Value(Some(val)) => {
                    let topic = match val {
                        ValueOrArray::Value(Some(val)) => Some(ValueOrArray::Value(val)),
                        ValueOrArray::Array(inner) => {
                            Some(ValueOrArray::Array(inner.into_iter().flatten().collect()))
                        }
                        _ => None,
                    };
                    if let Some(topic) = topic {
                        filter = filter.topic0(topic);
                    }
                }
                Topics::Array(topics) => {
                    for (idx, topic) in topics.into_iter().enumerate().take(4) {
                        if let Some(topic) = topic {
                            let topic = match topic {
                                ValueOrArray::Value(Some(val)) => ValueOrArray::Value(val),
                                ValueOrArray::Array(inner) => {
                                    ValueOrArray::Array(inner.into_iter().flatten().collect())
                                }
                                _ => continue,
                            };
                            filter = match idx {
                                0 => filter.topic0(topic),
                                1 => filter.topic1(topic),
                                2 => filter.topic2(topic),
                                3 => filter.topic3(topic),
                                _ => unreachable!(),
                            };
                        }
                    }
                }
                _ => {}
            }
        }

        filter
    }
}

/// Support for matching [Filter]s
#[derive(Debug, Default)]
pub struct FilteredParams {
    pub filter: Option<Filter>,
    pub flat_topics: Vec<ValueOrArray<Option<H256>>>,
}

impl FilteredParams {
    pub fn new(filter: Option<Filter>) -> Self {
        if let Some(filter) = filter {
            let flat_topics = filter.topics.as_ref().map(Self::flatten).unwrap_or_default();
            FilteredParams { filter: Some(filter), flat_topics }
        } else {
            Default::default()
        }
    }

    /// Returns the [BloomFilter] for the given address
    pub fn address_filter(address: &Option<ValueOrArray<Address>>) -> BloomFilter {
        address.as_ref().map(address_to_bloom_filter).unwrap_or_default()
    }

    /// Returns the [BloomFilter] for the given topics
    pub fn topics_filter(topics: &Option<Vec<ValueOrArray<Option<H256>>>>) -> Vec<BloomFilter> {
        let mut output = Vec::new();
        if let Some(topics) = topics {
            output.extend(topics.iter().map(topics_to_bloom_filter));
        }
        output
    }

    /// Returns `true` if the bloom matches the topics
    pub fn matches_topics(bloom: Bloom, topic_filters: &[BloomFilter]) -> bool {
        if topic_filters.is_empty() {
            return true
        }

        // returns true if a filter matches
        for filter in topic_filters.iter() {
            let mut is_match = false;
            for maybe_bloom in filter {
                is_match = maybe_bloom.as_ref().map(|b| bloom.contains_bloom(b)).unwrap_or(true);
                if !is_match {
                    break
                }
            }
            if is_match {
                return true
            }
        }
        false
    }

    /// Returns `true` if the bloom contains the address
    pub fn matches_address(bloom: Bloom, address_filter: &BloomFilter) -> bool {
        if address_filter.is_empty() {
            return true
        } else {
            for maybe_bloom in address_filter {
                if maybe_bloom.as_ref().map(|b| bloom.contains_bloom(b)).unwrap_or(true) {
                    return true
                }
            }
        }
        false
    }

    /// Flattens the topics using the cartesian product
    fn flatten(topic: &Topics) -> Vec<ValueOrArray<Option<H256>>> {
        fn cartesian(lists: &[Vec<Option<H256>>]) -> Vec<Vec<Option<H256>>> {
            let mut res = Vec::new();
            let mut list_iter = lists.iter();
            if let Some(first_list) = list_iter.next() {
                for &i in first_list {
                    res.push(vec![i]);
                }
            }
            for l in list_iter {
                let mut tmp = Vec::new();
                for r in res {
                    for &el in l {
                        let mut tmp_el = r.clone();
                        tmp_el.push(el);
                        tmp.push(tmp_el);
                    }
                }
                res = tmp;
            }
            res
        }
        let mut out = Vec::new();
        match topic {
            ValueOrArray::Array(multi) => {
                let mut tmp = Vec::new();
                for v in multi {
                    let v = if let Some(v) = v {
                        match v {
                            ValueOrArray::Value(s) => {
                                vec![*s]
                            }
                            ValueOrArray::Array(s) => s.clone(),
                        }
                    } else {
                        vec![None]
                    };
                    tmp.push(v);
                }
                for v in cartesian(&tmp) {
                    out.push(ValueOrArray::Array(v));
                }
            }
            ValueOrArray::Value(single) => {
                if let Some(single) = single {
                    out.push(single.clone());
                }
            }
        }
        out
    }

    /// Replace None values - aka wildcards - for the log input value in that position.
    pub fn replace(&self, log: &Log, topic: ValueOrArray<Option<H256>>) -> Option<Vec<H256>> {
        let mut out: Vec<H256> = Vec::new();
        match topic {
            ValueOrArray::Value(value) => {
                if let Some(value) = value {
                    out.push(value);
                }
            }
            ValueOrArray::Array(value) => {
                for (k, v) in value.into_iter().enumerate() {
                    if let Some(v) = v {
                        out.push(v);
                    } else {
                        out.push(log.topics[k]);
                    }
                }
            }
        };
        if out.is_empty() {
            return None
        }
        Some(out)
    }

    pub fn filter_block_range(&self, block_number: u64) -> bool {
        if self.filter.is_none() {
            return true
        }
        let filter = self.filter.as_ref().unwrap();
        let mut res = true;

        if let Some(BlockNumber::Number(num)) = filter.from_block {
            if num.as_u64() > block_number {
                res = false;
            }
        }

        if let Some(to) = filter.to_block {
            match to {
                BlockNumber::Number(num) => {
                    if num.as_u64() < block_number {
                        res = false;
                    }
                }
                BlockNumber::Earliest => {
                    res = false;
                }
                _ => {}
            }
        }
        res
    }

    pub fn filter_block_hash(&self, block_hash: H256) -> bool {
        if let Some(h) = self.filter.as_ref().and_then(|f| f.block_hash) {
            if h != block_hash {
                return false
            }
        }
        true
    }

    pub fn filter_address(&self, log: &Log) -> bool {
        if let Some(input_address) = &self.filter.as_ref().and_then(|f| f.address.clone()) {
            match input_address {
                ValueOrArray::Value(x) => {
                    if log.address != *x {
                        return false
                    }
                }
                ValueOrArray::Array(x) => {
                    if x.is_empty() {
                        return true
                    }
                    if !x.contains(&log.address) {
                        return false
                    }
                }
            }
        }
        true
    }

    pub fn filter_topics(&self, log: &Log) -> bool {
        let mut out: bool = true;
        for topic in self.flat_topics.iter().cloned() {
            match topic {
                ValueOrArray::Value(single) => {
                    if let Some(single) = single {
                        if !log.topics.starts_with(&[single]) {
                            out = false;
                        }
                    }
                }
                ValueOrArray::Array(multi) => {
                    if multi.is_empty() {
                        out = true;
                        continue
                    }
                    // Shrink the topics until the last item is Some.
                    let mut new_multi = multi;
                    while new_multi.iter().last().unwrap_or(&Some(H256::default())).is_none() {
                        new_multi.pop();
                    }
                    // We can discard right away any logs with lesser topics than the filter.
                    if new_multi.len() > log.topics.len() {
                        out = false;
                        break
                    }
                    let replaced: Option<Vec<H256>> =
                        self.replace(log, ValueOrArray::Array(new_multi));
                    if let Some(replaced) = replaced {
                        out = false;
                        if log.topics.starts_with(&replaced[..]) {
                            out = true;
                            break
                        }
                    }
                }
            }
        }
        out
    }
}

fn topics_to_bloom_filter(topics: &ValueOrArray<Option<H256>>) -> BloomFilter {
    let mut blooms = BloomFilter::new();
    match topics {
        ValueOrArray::Value(topic) => {
            if let Some(topic) = topic {
                let bloom: Bloom = BloomInput::Raw(topic.as_ref()).into();
                blooms.push(Some(bloom));
            } else {
                blooms.push(None);
            }
        }
        ValueOrArray::Array(topics) => {
            if topics.is_empty() {
                blooms.push(None);
            } else {
                for topic in topics.iter() {
                    if let Some(topic) = topic {
                        let bloom: Bloom = BloomInput::Raw(topic.as_ref()).into();
                        blooms.push(Some(bloom));
                    } else {
                        blooms.push(None);
                    }
                }
            }
        }
    }
    blooms
}

fn address_to_bloom_filter(address: &ValueOrArray<Address>) -> BloomFilter {
    let mut blooms = BloomFilter::new();
    match address {
        ValueOrArray::Value(address) => {
            let bloom: Bloom = BloomInput::Raw(address.as_ref()).into();
            blooms.push(Some(bloom))
        }
        ValueOrArray::Array(addresses) => {
            if addresses.is_empty() {
                blooms.push(None);
            } else {
                for address in addresses.iter() {
                    let bloom: Bloom = BloomInput::Raw(address.as_ref()).into();
                    blooms.push(Some(bloom));
                }
            }
        }
    }
    blooms
}

#[cfg(test)]
mod tests {

    use super::*;

    fn build_bloom(address: Address, topic1: H256, topic2: H256) -> Bloom {
        let mut block_bloom = Bloom::default();
        block_bloom.accrue(BloomInput::Raw(&address[..]));
        block_bloom.accrue(BloomInput::Raw(&topic1[..]));
        block_bloom.accrue(BloomInput::Raw(&topic2[..]));
        block_bloom
    }

    fn topic_filter(
        topic1: H256,
        topic2: H256,
        topic3: H256,
    ) -> (Filter, Option<Vec<ValueOrArray<Option<H256>>>>) {
        let filter = Filter {
            from_block: None,
            to_block: None,
            block_hash: None,
            address: None,
            topics: Some(ValueOrArray::Array(vec![
                Some(ValueOrArray::Value(Some(topic1))),
                Some(ValueOrArray::Array(vec![Some(topic2), Some(topic3)])),
            ])),
        };
        let topics = if filter.topics.is_some() {
            let filtered_params = FilteredParams::new(Some(filter.clone()));
            Some(filtered_params.flat_topics)
        } else {
            None
        };
        (filter, topics)
    }

    #[test]
    fn can_detect_different_topics() {
        let topic1 = H256::random();
        let topic2 = H256::random();
        let topic3 = H256::random();

        let (_, topics) = topic_filter(topic1, topic2, topic3);
        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(!FilteredParams::matches_topics(
            build_bloom(Address::random(), H256::random(), H256::random()),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_topic() {
        let topic1 = H256::random();
        let topic2 = H256::random();
        let topic3 = H256::random();

        let (_, topics) = topic_filter(topic1, topic2, topic3);
        let _topics_bloom = FilteredParams::topics_filter(&topics);

        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(FilteredParams::matches_topics(
            build_bloom(Address::random(), topic1, topic2),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_empty_topics() {
        let filter = Filter {
            from_block: None,
            to_block: None,
            block_hash: None,
            address: None,
            topics: Some(ValueOrArray::Array(vec![])),
        };
        let topics = if filter.topics.is_some() {
            let filtered_params = FilteredParams::new(Some(filter));
            Some(filtered_params.flat_topics)
        } else {
            None
        };
        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(FilteredParams::matches_topics(
            build_bloom(Address::random(), H256::random(), H256::random()),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_address_and_topics() {
        let rng_address = Address::random();
        let topic1 = H256::random();
        let topic2 = H256::random();
        let topic3 = H256::random();

        let filter = Filter {
            from_block: None,
            to_block: None,
            block_hash: None,
            address: Some(ValueOrArray::Value(rng_address)),
            topics: Some(ValueOrArray::Array(vec![
                Some(ValueOrArray::Value(Some(topic1))),
                Some(ValueOrArray::Array(vec![Some(topic2), Some(topic3)])),
            ])),
        };
        let topics = if filter.topics.is_some() {
            let filtered_params = FilteredParams::new(Some(filter.clone()));
            Some(filtered_params.flat_topics)
        } else {
            None
        };
        let address_filter = FilteredParams::address_filter(&filter.address);
        let topics_filter = FilteredParams::topics_filter(&topics);
        assert!(
            FilteredParams::matches_address(
                build_bloom(rng_address, topic1, topic2),
                &address_filter
            ) && FilteredParams::matches_topics(
                build_bloom(rng_address, topic1, topic2),
                &topics_filter
            )
        );
    }

    #[test]
    fn can_match_topics_wildcard() {
        let topic1 = H256::random();
        let topic2 = H256::random();
        let topic3 = H256::random();

        let filter = Filter {
            from_block: None,
            to_block: None,
            block_hash: None,
            address: None,
            topics: Some(ValueOrArray::Array(vec![
                None,
                Some(ValueOrArray::Array(vec![Some(topic2), Some(topic3)])),
            ])),
        };
        let topics = if filter.topics.is_some() {
            let filtered_params = FilteredParams::new(Some(filter));
            Some(filtered_params.flat_topics)
        } else {
            None
        };
        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(FilteredParams::matches_topics(
            build_bloom(Address::random(), topic1, topic2),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_topics_wildcard_mismatch() {
        let filter = Filter {
            from_block: None,
            to_block: None,
            block_hash: None,
            address: None,
            topics: Some(ValueOrArray::Array(vec![
                None,
                Some(ValueOrArray::Array(vec![Some(H256::random()), Some(H256::random())])),
            ])),
        };
        let topics_input = if filter.topics.is_some() {
            let filtered_params = FilteredParams::new(Some(filter));
            Some(filtered_params.flat_topics)
        } else {
            None
        };
        let topics_bloom = FilteredParams::topics_filter(&topics_input);
        assert!(!FilteredParams::matches_topics(
            build_bloom(Address::random(), H256::random(), H256::random()),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_address_filter() {
        let rng_address = Address::random();
        let filter = Filter {
            from_block: None,
            to_block: None,
            block_hash: None,
            address: Some(ValueOrArray::Value(rng_address)),
            topics: None,
        };
        let address_bloom = FilteredParams::address_filter(&filter.address);
        assert!(FilteredParams::matches_address(
            build_bloom(rng_address, H256::random(), H256::random(),),
            &address_bloom
        ));
    }

    #[test]
    fn can_detect_different_address() {
        let bloom_address = Address::random();
        let rng_address = Address::random();
        let filter = Filter {
            from_block: None,
            to_block: None,
            block_hash: None,
            address: Some(ValueOrArray::Value(rng_address)),
            topics: None,
        };
        let address_bloom = FilteredParams::address_filter(&filter.address);
        assert!(!FilteredParams::matches_address(
            build_bloom(bloom_address, H256::random(), H256::random(),),
            &address_bloom
        ));
    }
}
