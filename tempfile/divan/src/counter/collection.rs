use crate::counter::{AnyCounter, IntoCounter, KnownCounterKind, MaxCountUInt};

/// Multi-map from counters to their counts and input-based initializer.
#[derive(Default)]
pub(crate) struct CounterCollection {
    info: [KnownCounterInfo; KnownCounterKind::COUNT],
}

#[derive(Default)]
struct KnownCounterInfo {
    // TODO: Inlinable vector.
    counts: Vec<MaxCountUInt>,

    /// `BencherConfig::with_inputs` can only be called once, so the input type
    /// cannot change.
    count_input: Option<Box</* unsafe */ dyn Fn(*const ()) -> MaxCountUInt + Sync>>,
}

impl CounterCollection {
    #[inline]
    fn info(&self, counter_kind: KnownCounterKind) -> &KnownCounterInfo {
        &self.info[counter_kind as usize]
    }

    #[inline]
    fn info_mut(&mut self, counter_kind: KnownCounterKind) -> &mut KnownCounterInfo {
        &mut self.info[counter_kind as usize]
    }

    #[inline]
    pub(crate) fn counts(&self, counter_kind: KnownCounterKind) -> &[MaxCountUInt] {
        &self.info(counter_kind).counts
    }

    pub(crate) fn mean_count(&self, counter_kind: KnownCounterKind) -> MaxCountUInt {
        let counts = self.counts(counter_kind);

        let sum: u128 = counts.iter().map(|&c| c as u128).sum();

        (sum / counts.len() as u128) as MaxCountUInt
    }

    #[inline]
    pub(crate) fn uses_input_counts(&self, counter_kind: KnownCounterKind) -> bool {
        self.info(counter_kind).count_input.is_some()
    }

    pub(crate) fn set_counter(&mut self, counter: AnyCounter) {
        let new_count = counter.count();
        let info = self.info_mut(counter.known_kind());

        if let Some(old_count) = info.counts.first_mut() {
            *old_count = new_count;
        } else {
            info.counts.push(new_count);
        }
    }

    pub(crate) fn push_counter(&mut self, counter: AnyCounter) {
        self.info_mut(counter.known_kind()).counts.push(counter.count());
    }

    /// Set the input-based count generator function for a counter.
    pub(crate) fn set_input_counter<I, C, F>(&mut self, make_counter: F)
    where
        F: Fn(&I) -> C + Sync + 'static,
        C: IntoCounter,
    {
        let info = self.info_mut(KnownCounterKind::of::<C::Counter>());

        // Ignore previously-set counts.
        info.counts.clear();

        info.count_input = Some(Box::new(move |input: *const ()| {
            // SAFETY: Callers to `get_input_count` guarantee that the same `&I`
            // is passed.
            let counter = unsafe { make_counter(&*input.cast::<I>()) };

            AnyCounter::new(counter).count()
        }));
    }

    /// Calls the user-provided closure to get the counter count for a given
    /// input.
    ///
    /// # Safety
    ///
    /// The `I` type must be the same as that used by `set_input_counter`.
    pub(crate) unsafe fn get_input_count<I>(
        &self,
        counter_kind: KnownCounterKind,
        input: &I,
    ) -> Option<MaxCountUInt> {
        let from_input = self.info(counter_kind).count_input.as_ref()?;

        // SAFETY: The caller ensures that this is called on the same input type
        // used for calling `set_input_counter`.
        Some(unsafe { from_input(input as *const I as *const ()) })
    }

    /// Removes counts that came from input.
    pub(crate) fn clear_input_counts(&mut self) {
        for info in &mut self.info {
            if info.count_input.is_some() {
                info.counts.clear();
            }
        }
    }
}

/// A set of known and (future) custom counters.
#[derive(Clone, Debug, Default)]
pub struct CounterSet {
    counts: [Option<MaxCountUInt>; KnownCounterKind::COUNT],
}

impl CounterSet {
    pub fn with(mut self, counter: impl IntoCounter) -> Self {
        self.insert(counter);
        self
    }

    pub fn insert(&mut self, counter: impl IntoCounter) -> &mut Self {
        let counter = AnyCounter::new(counter);
        self.counts[counter.known_kind() as usize] = Some(counter.count());
        self
    }

    pub(crate) fn get(&self, counter_kind: KnownCounterKind) -> Option<MaxCountUInt> {
        self.counts[counter_kind as usize]
    }

    /// Overwrites `other` with values set in `self`.
    pub(crate) fn overwrite(&self, other: &Self) -> Self {
        Self { counts: KnownCounterKind::ALL.map(|kind| self.get(kind).or(other.get(kind))) }
    }

    pub(crate) fn to_collection(&self) -> CounterCollection {
        CounterCollection {
            info: KnownCounterKind::ALL.map(|kind| KnownCounterInfo {
                counts: self.get(kind).into_iter().collect(),
                count_input: None,
            }),
        }
    }
}
