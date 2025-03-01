//! Arbitrary implementations for `DynSolType` and `DynSolValue`.
//!
//! These implementations are guaranteed to be valid, including `CustomStruct`
//! identifiers.

// TODO: Maybe make array sizes configurable? Also change parameters type from
// tuple to a struct

// `prop_oneof!` / `TupleUnion` uses `Arc`s for cheap cloning
#![allow(clippy::arc_with_non_send_sync)]

use crate::{DynSolType, DynSolValue};
use alloy_primitives::{Address, Function, B256, I256, U256};
use arbitrary::{size_hint, Unstructured};
use core::ops::RangeInclusive;
use proptest::{
    collection::{vec as vec_strategy, VecStrategy},
    prelude::*,
    strategy::{Flatten, Map, Recursive, TupleUnion, WA},
};

const DEPTH: u32 = 16;
const DESIRED_SIZE: u32 = 64;
const EXPECTED_BRANCH_SIZE: u32 = 32;

macro_rules! prop_oneof_cfg {
    ($($(@[$attr:meta])* $w:expr => $x:expr,)+) => {
        TupleUnion::new(($(
            $(#[$attr])*
            {
                ($w as u32, ::alloc::sync::Arc::new($x))
            }
        ),+))
    };
}

#[cfg(not(feature = "eip712"))]
macro_rules! tuple_type_cfg {
    (($($t:ty),+ $(,)?), $c:ty $(,)?) => {
        ($($t,)+)
    };
}
#[cfg(feature = "eip712")]
macro_rules! tuple_type_cfg {
    (($($t:ty),+ $(,)?), $c:ty $(,)?) => {
        ($($t,)+ $c)
    };
}

#[inline]
const fn int_size(n: usize) -> usize {
    let n = (n % 255) + 1;
    n + (8 - (n % 8))
}

#[inline]
#[cfg(feature = "eip712")]
const fn ident_char(x: u8, first: bool) -> u8 {
    let x = x % 64;
    match x {
        0..=25 => x + b'a',
        26..=51 => (x - 26) + b'A',
        52 => b'_',
        53 => b'$',
        _ => {
            if first {
                b'a'
            } else {
                (x - 54) + b'0'
            }
        }
    }
}

fn non_empty_vec<'a, T: arbitrary::Arbitrary<'a>>(
    u: &mut Unstructured<'a>,
) -> arbitrary::Result<Vec<T>> {
    let sz = u.int_in_range(1..=16u8)?;
    let mut v = Vec::with_capacity(sz as usize);
    for _ in 0..sz {
        v.push(u.arbitrary()?);
    }
    Ok(v)
}

#[cfg(feature = "eip712")]
struct AString(String);

#[cfg(feature = "eip712")]
impl<'a> arbitrary::Arbitrary<'a> for AString {
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // note: do not use u.arbitrary() with String or Vec<u8> because it's always
        // too short
        let len = u.int_in_range(1..=128)?;
        let mut bytes = Vec::with_capacity(len);
        for i in 0..len {
            bytes.push(ident_char(u.arbitrary()?, i == 0));
        }
        Ok(Self::new(bytes))
    }

    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    fn arbitrary_take_rest(u: Unstructured<'a>) -> arbitrary::Result<Self> {
        let mut bytes = u.take_rest().to_owned();
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = ident_char(*byte, i == 0);
        }
        Ok(Self::new(bytes))
    }

    #[inline]
    fn size_hint(depth: usize) -> (usize, Option<usize>) {
        String::size_hint(depth)
    }
}

#[cfg(feature = "eip712")]
impl AString {
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    fn new(bytes: Vec<u8>) -> Self {
        debug_assert!(core::str::from_utf8(&bytes).is_ok());
        Self(unsafe { String::from_utf8_unchecked(bytes) })
    }
}

#[derive(Debug, derive_arbitrary::Arbitrary)]
enum Choice {
    Bool,
    Int,
    Uint,
    Address,
    Function,
    FixedBytes,
    Bytes,
    String,

    Array,
    FixedArray,
    Tuple,
    #[cfg(feature = "eip712")]
    CustomStruct,
}

impl<'a> arbitrary::Arbitrary<'a> for DynSolType {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        match u.arbitrary::<Choice>()? {
            Choice::Bool => Ok(Self::Bool),
            Choice::Int => u.arbitrary().map(int_size).map(Self::Int),
            Choice::Uint => u.arbitrary().map(int_size).map(Self::Uint),
            Choice::Address => Ok(Self::Address),
            Choice::Function => Ok(Self::Function),
            Choice::FixedBytes => Ok(Self::FixedBytes(u.int_in_range(1..=32)?)),
            Choice::Bytes => Ok(Self::Bytes),
            Choice::String => Ok(Self::String),
            Choice::Array => u.arbitrary().map(Self::Array),
            Choice::FixedArray => Ok(Self::FixedArray(u.arbitrary()?, u.int_in_range(1..=16)?)),
            Choice::Tuple => non_empty_vec(u).map(Self::Tuple),
            #[cfg(feature = "eip712")]
            Choice::CustomStruct => {
                let name = u.arbitrary::<AString>()?.0;
                let (prop_names, tuple) =
                    u.arbitrary_iter::<(AString, Self)>()?.flatten().map(|(a, b)| (a.0, b)).unzip();
                Ok(Self::CustomStruct { name, prop_names, tuple })
            }
        }
    }

    fn size_hint(depth: usize) -> (usize, Option<usize>) {
        if depth == DEPTH as usize {
            (0, Some(0))
        } else {
            size_hint::and(
                u32::size_hint(depth),
                size_hint::or_all(&[usize::size_hint(depth), Self::size_hint(depth + 1)]),
            )
        }
    }
}

impl<'a> arbitrary::Arbitrary<'a> for DynSolValue {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        match u.arbitrary::<DynSolType>()? {
            // re-use name and prop_names
            #[cfg(feature = "eip712")]
            DynSolType::CustomStruct { name, prop_names, tuple } => Ok(Self::CustomStruct {
                name,
                prop_names,
                tuple: tuple
                    .iter()
                    .map(|ty| Self::arbitrary_from_type(ty, u))
                    .collect::<Result<_, _>>()?,
            }),
            t => Self::arbitrary_from_type(&t, u),
        }
    }

    fn size_hint(depth: usize) -> (usize, Option<usize>) {
        if depth == DEPTH as usize {
            (0, Some(0))
        } else {
            size_hint::and(
                u32::size_hint(depth),
                size_hint::or_all(&[
                    B256::size_hint(depth),
                    usize::size_hint(depth),
                    Self::size_hint(depth + 1),
                ]),
            )
        }
    }
}

// rustscript
type ValueOfStrategy<S> = <S as Strategy>::Value;

type StratMap<S, T> = Map<S, fn(ValueOfStrategy<S>) -> T>;

type MappedWA<S, T> = WA<StratMap<S, T>>;

type Flat<S, T> = Flatten<StratMap<S, T>>;

type Rec<T, S> = Recursive<T, fn(BoxedStrategy<T>) -> S>;

#[cfg(feature = "eip712")]
const IDENT_STRATEGY: &str = parser::IDENT_REGEX;
#[cfg(feature = "eip712")]
type CustomStructStrategy<T> = BoxedStrategy<T>;

#[cfg(feature = "eip712")]
macro_rules! custom_struct_strategy {
    ($range:expr, $elem:expr) => {{
        // TODO: Avoid boxing. This is currently needed because we capture $elem
        let range: RangeInclusive<usize> = $range;
        let elem: BoxedStrategy<Self> = $elem;
        let strat: CustomStructStrategy<Self> = range
            .prop_flat_map(move |sz| {
                (
                    IDENT_STRATEGY,
                    proptest::collection::hash_set(IDENT_STRATEGY, sz..=sz)
                        .prop_map(|prop_names| prop_names.into_iter().collect()),
                    vec_strategy(elem.clone(), sz..=sz),
                )
            })
            .prop_map(|(name, prop_names, tuple)| Self::CustomStruct { name, prop_names, tuple })
            .boxed();
        strat
    }};
}

// we must explicitly the final types of the strategies
type TypeRecurseStrategy = TupleUnion<
    tuple_type_cfg![
        (
            WA<BoxedStrategy<DynSolType>>,                   // Basic
            MappedWA<BoxedStrategy<DynSolType>, DynSolType>, // Array
            MappedWA<(BoxedStrategy<DynSolType>, RangeInclusive<usize>), DynSolType>, // FixedArray
            MappedWA<VecStrategy<BoxedStrategy<DynSolType>>, DynSolType>, // Tuple
        ),
        WA<CustomStructStrategy<DynSolType>>, // CustomStruct
    ],
>;
type TypeStrategy = Rec<DynSolType, TypeRecurseStrategy>;

type ValueArrayStrategy =
    Flat<BoxedStrategy<DynSolValue>, VecStrategy<SBoxedStrategy<DynSolValue>>>;

type ValueRecurseStrategy = TupleUnion<
    tuple_type_cfg![
        (
            WA<BoxedStrategy<DynSolValue>>,            // Basic
            MappedWA<ValueArrayStrategy, DynSolValue>, // Array
            MappedWA<ValueArrayStrategy, DynSolValue>, // FixedArray
            MappedWA<VecStrategy<BoxedStrategy<DynSolValue>>, DynSolValue>, // Tuple
        ),
        WA<CustomStructStrategy<DynSolValue>>, // CustomStruct
    ],
>;
type ValueStrategy = Rec<DynSolValue, ValueRecurseStrategy>;

impl proptest::arbitrary::Arbitrary for DynSolType {
    type Parameters = (u32, u32, u32);
    type Strategy = TypeStrategy;

    #[inline]
    fn arbitrary() -> Self::Strategy {
        Self::arbitrary_with((DEPTH, DESIRED_SIZE, EXPECTED_BRANCH_SIZE))
    }

    fn arbitrary_with(args: Self::Parameters) -> Self::Strategy {
        let (depth, desired_size, expected_branch_size) = args;
        Self::leaf().prop_recursive(depth, desired_size, expected_branch_size, Self::recurse)
    }
}

impl DynSolType {
    /// Generate an arbitrary [`DynSolValue`] from this type.
    #[inline]
    pub fn arbitrary_value(&self, u: &mut Unstructured<'_>) -> arbitrary::Result<DynSolValue> {
        DynSolValue::arbitrary_from_type(self, u)
    }

    /// Create a [proptest strategy][Strategy] to generate [`DynSolValue`]s from
    /// this type.
    #[inline]
    pub fn value_strategy(&self) -> SBoxedStrategy<DynSolValue> {
        DynSolValue::type_strategy(self)
    }

    #[inline]
    fn leaf() -> impl Strategy<Value = Self> {
        prop_oneof![
            Just(Self::Bool),
            Just(Self::Address),
            any::<usize>().prop_map(|x| Self::Int(int_size(x))),
            any::<usize>().prop_map(|x| Self::Uint(int_size(x))),
            (1..=32usize).prop_map(Self::FixedBytes),
            Just(Self::Bytes),
            Just(Self::String),
        ]
    }

    #[inline]
    fn recurse(element: BoxedStrategy<Self>) -> TypeRecurseStrategy {
        prop_oneof_cfg![
            1 => element.clone(),
            2 => element.clone().prop_map(|ty| Self::Array(Box::new(ty))),
            2 => (element.clone(), 1..=16).prop_map(|(ty, sz)| Self::FixedArray(Box::new(ty), sz)),
            2 => vec_strategy(element.clone(), 1..=16).prop_map(Self::Tuple),
            @[cfg(feature = "eip712")]
            1 => custom_struct_strategy!(1..=16, element),
        ]
    }
}

impl proptest::arbitrary::Arbitrary for DynSolValue {
    type Parameters = (u32, u32, u32);
    type Strategy = ValueStrategy;

    #[inline]
    fn arbitrary() -> Self::Strategy {
        Self::arbitrary_with((DEPTH, DESIRED_SIZE, EXPECTED_BRANCH_SIZE))
    }

    fn arbitrary_with(args: Self::Parameters) -> Self::Strategy {
        let (depth, desired_size, expected_branch_size) = args;
        Self::leaf().prop_recursive(depth, desired_size, expected_branch_size, Self::recurse)
    }
}

impl DynSolValue {
    /// Generate an arbitrary [`DynSolValue`] from the given [`DynSolType`].
    pub fn arbitrary_from_type(
        ty: &DynSolType,
        u: &mut Unstructured<'_>,
    ) -> arbitrary::Result<Self> {
        match ty {
            DynSolType::Bool => u.arbitrary().map(Self::Bool),
            DynSolType::Address => u.arbitrary().map(Self::Address),
            DynSolType::Function => u.arbitrary().map(Self::Function),
            &DynSolType::Int(sz) => u.arbitrary().map(|x| Self::Int(adjust_int(x, sz), sz)),
            &DynSolType::Uint(sz) => u.arbitrary().map(|x| Self::Uint(adjust_uint(x, sz), sz)),
            &DynSolType::FixedBytes(sz) => {
                u.arbitrary().map(|x| Self::FixedBytes(adjust_fb(x, sz), sz))
            }
            DynSolType::Bytes => u.arbitrary().map(Self::Bytes),
            DynSolType::String => u.arbitrary().map(Self::String),
            DynSolType::Array(ty) => {
                let sz = u.int_in_range(1..=16u8)?;
                let mut v = Vec::with_capacity(sz as usize);
                for _ in 0..sz {
                    v.push(Self::arbitrary_from_type(ty, u)?);
                }
                Ok(Self::Array(v))
            }
            &DynSolType::FixedArray(ref ty, sz) => {
                let mut v = Vec::with_capacity(sz);
                for _ in 0..sz {
                    v.push(Self::arbitrary_from_type(ty, u)?);
                }
                Ok(Self::FixedArray(v))
            }
            DynSolType::Tuple(tuple) => tuple
                .iter()
                .map(|ty| Self::arbitrary_from_type(ty, u))
                .collect::<Result<Vec<_>, _>>()
                .map(Self::Tuple),
            #[cfg(feature = "eip712")]
            DynSolType::CustomStruct { tuple, .. } => {
                let name = u.arbitrary::<AString>()?.0;
                let tuple = tuple
                    .iter()
                    .map(|ty| Self::arbitrary_from_type(ty, u))
                    .collect::<Result<Vec<_>, _>>()?;
                let sz = tuple.len();
                let prop_names = (0..sz)
                    .map(|_| u.arbitrary::<AString>().map(|s| s.0))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Self::CustomStruct { name, prop_names, tuple })
            }
        }
    }

    /// Create a [proptest strategy][Strategy] to generate [`DynSolValue`]s from
    /// the given type.
    pub fn type_strategy(ty: &DynSolType) -> SBoxedStrategy<Self> {
        match ty {
            DynSolType::Bool => any::<bool>().prop_map(Self::Bool).sboxed(),
            DynSolType::Address => any::<Address>().prop_map(Self::Address).sboxed(),
            DynSolType::Function => any::<Function>().prop_map(Self::Function).sboxed(),
            &DynSolType::Int(sz) => {
                any::<I256>().prop_map(move |x| Self::Int(adjust_int(x, sz), sz)).sboxed()
            }
            &DynSolType::Uint(sz) => {
                any::<U256>().prop_map(move |x| Self::Uint(adjust_uint(x, sz), sz)).sboxed()
            }
            &DynSolType::FixedBytes(sz) => {
                any::<B256>().prop_map(move |x| Self::FixedBytes(adjust_fb(x, sz), sz)).sboxed()
            }
            DynSolType::Bytes => any::<Vec<u8>>().prop_map(Self::Bytes).sboxed(),
            DynSolType::String => any::<String>().prop_map(Self::String).sboxed(),
            DynSolType::Array(ty) => {
                let element = Self::type_strategy(ty);
                vec_strategy(element, 1..=16).prop_map(Self::Array).sboxed()
            }
            DynSolType::FixedArray(ty, sz) => {
                let element = Self::type_strategy(ty);
                vec_strategy(element, *sz).prop_map(Self::FixedArray).sboxed()
            }
            DynSolType::Tuple(tys) => tys
                .iter()
                .map(Self::type_strategy)
                .collect::<Vec<_>>()
                .prop_map(Self::Tuple)
                .sboxed(),
            #[cfg(feature = "eip712")]
            DynSolType::CustomStruct { tuple, prop_names, name } => {
                let name = name.clone();
                let prop_names = prop_names.clone();
                tuple
                    .iter()
                    .map(Self::type_strategy)
                    .collect::<Vec<_>>()
                    .prop_map(move |tuple| Self::CustomStruct {
                        name: name.clone(),
                        prop_names: prop_names.clone(),
                        tuple,
                    })
                    .sboxed()
            }
        }
    }

    /// Create a [proptest strategy][Strategy] to generate [`DynSolValue`]s from
    /// the given value's type.
    #[inline]
    pub fn value_strategy(&self) -> SBoxedStrategy<Self> {
        Self::type_strategy(&self.as_type().unwrap())
    }

    #[inline]
    fn leaf() -> impl Strategy<Value = Self> {
        prop_oneof![
            any::<bool>().prop_map(Self::Bool),
            any::<Address>().prop_map(Self::Address),
            int_strategy::<I256>().prop_map(|(x, sz)| Self::Int(adjust_int(x, sz), sz)),
            int_strategy::<U256>().prop_map(|(x, sz)| Self::Uint(adjust_uint(x, sz), sz)),
            (any::<B256>(), 1..=32usize).prop_map(|(x, sz)| Self::FixedBytes(adjust_fb(x, sz), sz)),
            any::<Vec<u8>>().prop_map(Self::Bytes),
            any::<String>().prop_map(Self::String),
        ]
    }

    #[inline]
    fn recurse(element: BoxedStrategy<Self>) -> ValueRecurseStrategy {
        prop_oneof_cfg![
            1 => element.clone(),
            2 => Self::array_strategy(element.clone()).prop_map(Self::Array),
            2 => Self::array_strategy(element.clone()).prop_map(Self::FixedArray),
            2 => vec_strategy(element.clone(), 1..=16).prop_map(Self::Tuple),
            @[cfg(feature = "eip712")]
            1 => custom_struct_strategy!(1..=16, element),
        ]
    }

    /// Recursive array strategy that generates same-type arrays of up to 16
    /// elements.
    ///
    /// NOTE: this has to be a separate function so Rust can turn the closure
    /// type (`impl Fn`) into an `fn` type.
    ///
    /// If you manually inline this into the function above, the compiler will
    /// fail with "expected fn pointer, found closure":
    ///
    /// ```ignore (error)
    ///    error[E0308]: mismatched types
    ///    --> crates/dyn-abi/src/arbitrary.rs:264:18
    ///     |
    /// 261 | /         prop_oneof![
    /// 262 | |             1 => element.clone(),
    /// 263 | |             2 => Self::array_strategy(element.clone()).prop_map(Self::Array),
    /// 264 | |             2 => element.prop_flat_map(|x| vec_strategy(x.value_strategy(), 1..=16)).prop_map(Self::FixedArray),
    ///     | |                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected fn pointer, found closure
    /// 265 | |             2 => vec_strategy(element, 1..=16).prop_map(Self::Tuple),
    /// 266 | |         ]
    ///     | |_________- arguments to this function are incorrect
    ///     |
    ///     = note: expected struct `Map<Flatten<Map<BoxedStrategy<DynSolValue>, fn(DynSolValue) -> VecStrategy<BoxedStrategy<DynSolValue>>>>, ...>`
    ///                found struct `Map<Flatten<Map<BoxedStrategy<DynSolValue>, [closure@arbitrary.rs:264:40]>>, ...>`
    /// ```
    #[inline]
    #[allow(rustdoc::invalid_rust_codeblocks)]
    fn array_strategy(element: BoxedStrategy<Self>) -> ValueArrayStrategy {
        element.prop_flat_map(|x| vec_strategy(x.value_strategy(), 1..=16))
    }
}

#[inline]
fn int_strategy<T: Arbitrary>() -> impl Strategy<Value = (ValueOfStrategy<T::Strategy>, usize)> {
    (any::<T>(), any::<usize>().prop_map(int_size))
}

// Trim words and integers to the given size.
#[inline]
fn adjust_int(mut int: I256, size: usize) -> I256 {
    if size < 256 {
        if int.bit(size - 1) {
            int |= I256::MINUS_ONE - (I256::ONE << size).wrapping_sub(I256::ONE);
        } else {
            int &= (I256::ONE << size).wrapping_sub(I256::ONE);
        }
    }
    int
}

#[inline]
fn adjust_uint(mut uint: U256, size: usize) -> U256 {
    if size < 256 {
        uint &= (U256::from(1u64) << size).wrapping_sub(U256::from(1u64));
    }
    uint
}

#[inline]
fn adjust_fb(mut word: B256, size: usize) -> B256 {
    if size < 32 {
        word[size..].fill(0);
    }
    word
}

#[cfg(all(test, not(miri)))] // doesn't run in isolation and would take too long
mod tests {
    use super::*;
    #[cfg(feature = "eip712")]
    use parser::{is_id_continue, is_id_start, is_valid_identifier};

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 1024,
            ..Default::default()
        })]

        #[test]
        fn int_size(x: usize) {
            let sz = super::int_size(x);
            prop_assert!(sz > 0 && sz <= 256, "{sz}");
            prop_assert!(sz % 8 == 0, "{sz}");
        }

        #[test]
        #[cfg(feature = "eip712")]
        fn ident_char(x: u8) {
            let start = super::ident_char(x, true);
            prop_assert!(is_id_start(start as char));
            prop_assert!(is_id_continue(start as char));

            let cont = super::ident_char(x, false);
            prop_assert!(is_id_continue(cont as char));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            ..Default::default()
        })]

        #[test]
        #[cfg(feature = "eip712")]
        fn arbitrary_string(bytes: Vec<u8>) {
            prop_assume!(!bytes.is_empty());
            let mut u = Unstructured::new(&bytes);

            let s = u.arbitrary::<AString>();
            prop_assume!(s.is_ok());

            let s = s.unwrap().0;
            prop_assume!(!s.is_empty());

            prop_assert!(
                is_valid_identifier(&s),
                "not a valid identifier: {:?}\ndata: {}",
                s,
                hex::encode_prefixed(&bytes),
            );
        }

        #[test]
        fn arbitrary_type(bytes: Vec<u8>) {
            prop_assume!(!bytes.is_empty());
            let mut u = Unstructured::new(&bytes);
            let ty = u.arbitrary::<DynSolType>();
            prop_assume!(ty.is_ok());
            type_test(ty.unwrap())?;
        }

        #[test]
        fn arbitrary_value(bytes: Vec<u8>) {
            prop_assume!(!bytes.is_empty());
            let mut u = Unstructured::new(&bytes);
            let value = u.arbitrary::<DynSolValue>();
            prop_assume!(value.is_ok());
            value_test(value.unwrap())?;
        }

        #[test]
        fn proptest_type(ty: DynSolType) {
            type_test(ty)?;
        }

        #[test]
        fn proptest_value(value: DynSolValue) {
            value_test(value)?;
        }
    }

    fn type_test(ty: DynSolType) -> Result<(), TestCaseError> {
        let s = ty.sol_type_name();
        prop_assume!(!ty.has_custom_struct());
        prop_assert_eq!(DynSolType::parse(&s), Ok(ty), "type strings don't match");
        Ok(())
    }

    fn value_test(value: DynSolValue) -> Result<(), TestCaseError> {
        let ty = match value.as_type() {
            Some(ty) => ty,
            None => {
                prop_assert!(false, "generated invalid type: {value:?}");
                unreachable!()
            }
        };
        // this shouldn't fail after the previous assertion
        let s = value.sol_type_name().unwrap();

        prop_assert_eq!(&s, &ty.sol_type_name(), "type strings don't match");

        assert_valid_value(&value)?;

        // allow this to fail if the type contains a CustomStruct
        if !ty.has_custom_struct() {
            let parsed = s.parse::<DynSolType>();
            prop_assert_eq!(parsed.as_ref(), Ok(&ty), "types don't match {:?}", s);
        }

        let data = value.abi_encode_params();
        match ty.abi_decode_params(&data) {
            // skip the check if the type contains a CustomStruct, since
            // decoding will not populate names
            Ok(decoded) if !decoded.has_custom_struct() => prop_assert_eq!(
                &decoded,
                &value,
                "\n\ndecoded value doesn't match {:?} ({:?})\ndata: {:?}",
                s,
                ty,
                hex::encode_prefixed(&data),
            ),
            Ok(_) => {}
            Err(e @ crate::Error::SolTypes(alloy_sol_types::Error::RecursionLimitExceeded(_))) => {
                return Err(TestCaseError::Reject(e.to_string().into()));
            }
            Err(e) => prop_assert!(
                false,
                "failed to decode {s:?}: {e}\nvalue: {value:?}\ndata: {:?}",
                hex::encode_prefixed(&data),
            ),
        }

        Ok(())
    }

    pub(crate) fn assert_valid_value(value: &DynSolValue) -> Result<(), TestCaseError> {
        match &value {
            DynSolValue::Array(values) | DynSolValue::FixedArray(values) => {
                prop_assert!(!values.is_empty());
                let mut values = values.iter();
                let ty = values.next().unwrap().as_type().unwrap();
                prop_assert!(
                    values.all(|v| ty.matches(v)),
                    "array elements have different types: {value:#?}",
                );
            }
            #[cfg(feature = "eip712")]
            DynSolValue::CustomStruct { name, prop_names, tuple } => {
                prop_assert!(is_valid_identifier(name));
                prop_assert!(prop_names.iter().all(|s| is_valid_identifier(s)));
                prop_assert_eq!(prop_names.len(), tuple.len());
            }
            _ => {}
        }

        match value {
            DynSolValue::Int(int, size) => {
                let bits = int.into_sign_and_abs().1.bit_len();
                prop_assert!(bits <= *size, "int: {int}, {size}, {bits}")
            }
            DynSolValue::Uint(uint, size) => {
                let bits = uint.bit_len();
                prop_assert!(bits <= *size, "uint: {uint}, {size}, {bits}")
            }
            DynSolValue::FixedBytes(fb, size) => {
                prop_assert!(fb[*size..].iter().all(|x| *x == 0), "fb {fb}, {size}")
            }
            _ => {}
        }

        // recurse
        match value {
            DynSolValue::Array(t)
            | DynSolValue::FixedArray(t)
            | crate::dynamic::ty::as_tuple!(DynSolValue t) => {
                t.iter().try_for_each(assert_valid_value)?
            }
            _ => {}
        }

        Ok(())
    }
}
