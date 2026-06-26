use crate::{
    FuzzFixtures,
    strategies::{FuzzStateReader, fuzz_param_from_state, fuzz_param_with_fixtures},
};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::{Function, Param};
use alloy_primitives::{Bytes, U256};
use proptest::{prelude::Strategy, strategy::BoxedStrategy};

/// Plan for constraining the enum leaves of a fuzzed parameter into their valid `0..variant_count`
/// range. Solidity enums are ABI-encoded as `uint8`, so without this the fuzzer can generate
/// out-of-range values that the contract rejects with `Panic(0x21)` when decoding them.
#[derive(Clone, Debug)]
enum EnumClamp {
    /// An enum (possibly nested in arrays); reduce every `uint8` leaf modulo the variant count.
    Leaf(U256),
    /// A tuple/struct; apply the per-component plans (`None` = no clamping needed).
    Tuple(Vec<Option<Self>>),
}

impl EnumClamp {
    /// Builds a clamp plan for `input`, returning `None` if it contains no enums to constrain.
    fn for_param(input: &Param, fuzz_fixtures: &FuzzFixtures) -> Option<Self> {
        // Direct enum, e.g. `EnumVal` or `EnumVal[2][]`; strip any array suffix.
        if let Some((contract, ty)) = input.internal_type.as_ref().and_then(|it| it.as_enum()) {
            let base = ty.split('[').next().unwrap_or(ty);
            if let Some(count) = fuzz_fixtures.enum_variant_count(contract, base)
                && count > 0
            {
                return Some(Self::Leaf(U256::from(count)));
            }
        }

        // Struct/tuple (or `Struct[]`): recurse into components, keeping the plan only if any field
        // needs clamping.
        if input.components.is_empty() {
            return None;
        }
        let fields = input
            .components
            .iter()
            .map(|component| Self::for_param(component, fuzz_fixtures))
            .collect::<Vec<_>>();
        fields.iter().any(Option::is_some).then_some(Self::Tuple(fields))
    }

    /// Applies the plan to a generated value, reducing every enum leaf into its valid range.
    fn apply(&self, value: DynSolValue) -> DynSolValue {
        match self {
            Self::Leaf(count) => clamp_enum_leaf(value, *count),
            Self::Tuple(fields) => match value {
                DynSolValue::Tuple(values) => DynSolValue::Tuple(self.apply_fields(fields, values)),
                DynSolValue::CustomStruct { name, prop_names, tuple } => {
                    DynSolValue::CustomStruct {
                        name,
                        prop_names,
                        tuple: self.apply_fields(fields, tuple),
                    }
                }
                // `Struct[]`/`Struct[N]`: apply the field plan to each element.
                DynSolValue::Array(values) => {
                    DynSolValue::Array(values.into_iter().map(|v| self.apply(v)).collect())
                }
                DynSolValue::FixedArray(values) => {
                    DynSolValue::FixedArray(values.into_iter().map(|v| self.apply(v)).collect())
                }
                other => other,
            },
        }
    }

    fn apply_fields(&self, fields: &[Option<Self>], values: Vec<DynSolValue>) -> Vec<DynSolValue> {
        values
            .into_iter()
            .enumerate()
            .map(|(i, value)| match fields.get(i) {
                Some(Some(plan)) => plan.apply(value),
                _ => value,
            })
            .collect()
    }
}

/// Wraps `strat` to constrain any enum leaves in `input` to their valid range; a no-op otherwise.
fn bound_enum(
    strat: BoxedStrategy<DynSolValue>,
    input: &Param,
    fuzz_fixtures: &FuzzFixtures,
) -> BoxedStrategy<DynSolValue> {
    match EnumClamp::for_param(input, fuzz_fixtures) {
        Some(plan) => strat.prop_map(move |value| plan.apply(value)).boxed(),
        None => strat,
    }
}

/// Recursively reduces every `uint8` leaf in an enum value (possibly nested in arrays) modulo
/// `count`.
fn clamp_enum_leaf(value: DynSolValue, count: U256) -> DynSolValue {
    match value {
        DynSolValue::Uint(v, 8) => DynSolValue::Uint(v % count, 8),
        DynSolValue::Array(values) => {
            DynSolValue::Array(values.into_iter().map(|v| clamp_enum_leaf(v, count)).collect())
        }
        DynSolValue::FixedArray(values) => {
            DynSolValue::FixedArray(values.into_iter().map(|v| clamp_enum_leaf(v, count)).collect())
        }
        other => other,
    }
}

/// Given a function, it returns a strategy which generates valid calldata
/// for that function's input types, following declared test fixtures.
pub fn fuzz_calldata(
    func: Function,
    fuzz_fixtures: &FuzzFixtures,
) -> impl Strategy<Value = Bytes> + use<> {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations, accounting any parameter declared fixture
    let strats = func
        .inputs
        .iter()
        .map(|input| {
            let strat = fuzz_param_with_fixtures(
                &input.selector_type().parse().unwrap(),
                fuzz_fixtures.param_fixtures(&input.name),
                &input.name,
            );
            bound_enum(strat, input, fuzz_fixtures)
        })
        .collect::<Vec<_>>();
    strats.prop_map(move |values| {
        func.abi_encode_input(&values)
            .unwrap_or_else(|_| {
                panic!(
                    "Fuzzer generated invalid arguments for function `{}` with inputs {:?}: {:?}",
                    func.name, func.inputs, values
                )
            })
            .into()
    })
}

/// Given a function and some state, it returns a strategy which generated valid calldata for the
/// given function's input types, based on state taken from the EVM.
pub fn fuzz_calldata_from_state<S: FuzzStateReader>(
    func: Function,
    state: &S,
    fuzz_fixtures: &FuzzFixtures,
) -> impl Strategy<Value = Bytes> + use<S> {
    let strats = func
        .inputs
        .iter()
        .map(|input| {
            let strat = fuzz_param_from_state(&input.selector_type().parse().unwrap(), state);
            bound_enum(strat, input, fuzz_fixtures)
        })
        .collect::<Vec<_>>();
    strats
        .prop_map(move |values| {
            func.abi_encode_input(&values)
                .unwrap_or_else(|_| {
                    panic!(
                        "Fuzzer generated invalid arguments for function `{}` with inputs {:?}: {:?}",
                        func.name, func.inputs, values
                    )
                })
                .into()
        })
        .no_shrink()
}

#[cfg(test)]
mod tests {
    use crate::{FuzzFixtures, strategies::fuzz_calldata};
    use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
    use alloy_json_abi::Function;
    use alloy_primitives::{Address, map::HashMap};
    use proptest::prelude::Strategy;

    #[test]
    fn can_fuzz_with_fixtures() {
        let function = Function::parse("test_fuzzed_address(address addressFixture)").unwrap();

        let address_fixture = DynSolValue::Address(Address::random());
        let mut fixtures = HashMap::default();
        fixtures.insert(
            "addressFixture".to_string(),
            DynSolValue::Array(vec![address_fixture.clone()]),
        );

        let expected = function.abi_encode_input(&[address_fixture]).unwrap();
        let strategy = fuzz_calldata(function, &FuzzFixtures::new(fixtures));
        let _ = strategy.prop_map(move |fuzzed| {
            assert_eq!(expected, fuzzed);
        });
    }
}
