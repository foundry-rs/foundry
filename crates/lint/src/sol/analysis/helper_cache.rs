use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
};

pub const DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT: usize = 65_536;

/// Bounded memo table for lint analyses that inline internal helper calls.
#[derive(Debug)]
pub struct HelperAnalysisCache<K, V> {
    entries: HashMap<K, V>,
    in_progress: HashSet<K>,
    order: VecDeque<K>,
    max_entries: usize,
}

impl<K, V> HelperAnalysisCache<K, V>
where
    K: Clone + Eq + Hash,
{
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            in_progress: HashSet::new(),
            order: VecDeque::new(),
            max_entries,
        }
    }

    pub fn is_in_progress(&self, key: &K) -> bool {
        self.in_progress.contains(key)
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    pub fn start(&mut self, key: K) {
        self.in_progress.insert(key);
    }

    pub fn finish(&mut self, key: K, value: V) {
        self.in_progress.remove(&key);
        if self.max_entries == 0 {
            return;
        }

        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.entries.insert(key, value);

        while self.entries.len() > self.max_entries {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
                self.in_progress.remove(&oldest);
            } else {
                break;
            }
        }
    }
}
