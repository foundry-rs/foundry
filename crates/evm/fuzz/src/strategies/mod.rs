use proptest::prelude::prop;

mod int;
pub use int::IntStrategy;

mod uint;
pub use uint::UintStrategy;

mod param;
pub use param::{fuzz_msg_value, fuzz_param, fuzz_param_with_fixtures, generate_msg_value};
pub(crate) use param::{fuzz_param_from_state, mutate_param_value};

mod calldata;
pub use calldata::fuzz_calldata;
pub(crate) use calldata::{constrain_enum_value, fuzz_calldata_from_state};

mod state;
pub(crate) use state::DictionaryRead;
pub use state::{EvmFuzzState, FuzzState};

mod invariants;
pub use invariants::override_call_strat;

mod tx;
pub use tx::TxGenerator;

mod mutators;
pub use mutators::BoundMutator;

mod literals;
pub use literals::{EnumBounds, LiteralMaps, LiteralsCollector, LiteralsDictionary};

#[derive(Clone, Debug, Default)]
struct WeightedIndices {
    cumulative: Vec<u64>,
}

impl WeightedIndices {
    fn from_weights(weights: impl IntoIterator<Item = u32>) -> Self {
        let mut total = 0u64;
        let cumulative = weights
            .into_iter()
            .map(|weight| {
                total += u64::from(weight);
                total
            })
            .collect();
        Self { cumulative }
    }

    /// Selects by weight, falling back to uniform selection when every weight is zero.
    fn select(&self, index: prop::sample::Index, len: usize) -> usize {
        match self.cumulative.last() {
            Some(&total) if total > 0 => {
                let point = index.index(total as usize) as u64;
                self.cumulative.partition_point(|&weight| weight <= point)
            }
            _ => index.index(len),
        }
    }
}
