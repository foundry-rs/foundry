use crate::{alloc::string::ToString, Error, Panic, Result, Revert, SolError};
use alloc::{string::String, vec::Vec};
use core::{convert::Infallible, fmt, iter::FusedIterator, marker::PhantomData};

mod event;
pub use event::SolEventInterface;

/// A collection of ABI-encodable call-like types. This currently includes
/// [`SolCall`] and [`SolError`].
///
/// This trait assumes that the implementing type always has a selector, and
/// thus encoded/decoded data is always at least 4 bytes long.
///
/// This trait is implemented for [`Infallible`] to represent an empty
/// interface. This is used by [`GenericContractError`].
///
/// [`SolCall`]: crate::SolCall
/// [`SolError`]: crate::SolError
///
/// # Implementer's Guide
///
/// It should not be necessary to implement this trait manually. Instead, use
/// the [`sol!`](crate::sol!) procedural macro to parse Solidity syntax into
/// types that implement this trait.
pub trait SolInterface: Sized {
    /// The name of this type.
    const NAME: &'static str;

    /// The minimum length of the data for this type.
    ///
    /// This does *not* include the selector's length (4).
    const MIN_DATA_LENGTH: usize;

    /// The number of variants.
    const COUNT: usize;

    /// The selector of this instance.
    fn selector(&self) -> [u8; 4];

    /// The selector of this type at the given index, used in
    /// [`selectors`](Self::selectors).
    ///
    /// This **must** return `None` if `i >= Self::COUNT`, and `Some` with a
    /// different selector otherwise.
    fn selector_at(i: usize) -> Option<[u8; 4]>;

    /// Returns `true` if the given selector is known to this type.
    fn valid_selector(selector: [u8; 4]) -> bool;

    /// Returns an error if the given selector is not known to this type.
    fn type_check(selector: [u8; 4]) -> Result<()> {
        if Self::valid_selector(selector) {
            Ok(())
        } else {
            Err(Error::UnknownSelector { name: Self::NAME, selector: selector.into() })
        }
    }

    /// ABI-decodes the given data into one of the variants of `self`.
    fn abi_decode_raw(selector: [u8; 4], data: &[u8], validate: bool) -> Result<Self>;

    /// The size of the encoded data, *without* any selectors.
    fn abi_encoded_size(&self) -> usize;

    /// ABI-encodes `self` into the given buffer, *without* any selectors.
    fn abi_encode_raw(&self, out: &mut Vec<u8>);

    /// Returns an iterator over the selectors of this type.
    #[inline]
    fn selectors() -> Selectors<Self> {
        Selectors::new()
    }

    /// ABI-encodes `self` into the given buffer.
    #[inline]
    fn abi_encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.abi_encoded_size());
        out.extend(self.selector());
        self.abi_encode_raw(&mut out);
        out
    }

    /// ABI-decodes the given data into one of the variants of `self`.
    #[inline]
    fn abi_decode(data: &[u8], validate: bool) -> Result<Self> {
        if data.len() < Self::MIN_DATA_LENGTH.saturating_add(4) {
            Err(crate::Error::type_check_fail(data, Self::NAME))
        } else {
            let (selector, data) = data.split_first_chunk().unwrap();
            Self::abi_decode_raw(*selector, data, validate)
        }
    }
}

/// An empty [`SolInterface`] implementation. Used by [`GenericContractError`].
impl SolInterface for Infallible {
    // better than "Infallible" since it shows up in error messages
    const NAME: &'static str = "GenericContractError";

    // no selectors or data are valid
    const MIN_DATA_LENGTH: usize = usize::MAX;
    const COUNT: usize = 0;

    #[inline]
    fn selector(&self) -> [u8; 4] {
        unreachable!()
    }

    #[inline]
    fn selector_at(_i: usize) -> Option<[u8; 4]> {
        None
    }

    #[inline]
    fn valid_selector(_selector: [u8; 4]) -> bool {
        false
    }

    #[inline]
    fn abi_decode_raw(selector: [u8; 4], _data: &[u8], _validate: bool) -> Result<Self> {
        Self::type_check(selector).map(|()| unreachable!())
    }

    #[inline]
    fn abi_encoded_size(&self) -> usize {
        unreachable!()
    }

    #[inline]
    fn abi_encode_raw(&self, _out: &mut Vec<u8>) {
        unreachable!()
    }
}

/// A generic contract error.
///
/// Contains a [`Revert`] or [`Panic`] error.
pub type GenericContractError = ContractError<Infallible>;

/// A generic contract error.
///
/// Contains a [`Revert`] or [`Panic`] error, or a custom error.
///
/// If you want an empty [`CustomError`](ContractError::CustomError) variant,
/// use [`GenericContractError`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractError<T> {
    /// A contract's custom error.
    CustomError(T),
    /// A generic revert. See [`Revert`] for more information.
    Revert(Revert),
    /// A panic. See [`Panic`] for more information.
    Panic(Panic),
}

impl<T: SolInterface> From<T> for ContractError<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self::CustomError(value)
    }
}

impl<T> From<Revert> for ContractError<T> {
    #[inline]
    fn from(value: Revert) -> Self {
        Self::Revert(value)
    }
}

impl<T> TryFrom<ContractError<T>> for Revert {
    type Error = ContractError<T>;

    #[inline]
    fn try_from(value: ContractError<T>) -> Result<Self, Self::Error> {
        match value {
            ContractError::Revert(inner) => Ok(inner),
            _ => Err(value),
        }
    }
}

impl<T> From<Panic> for ContractError<T> {
    #[inline]
    fn from(value: Panic) -> Self {
        Self::Panic(value)
    }
}

impl<T> TryFrom<ContractError<T>> for Panic {
    type Error = ContractError<T>;

    #[inline]
    fn try_from(value: ContractError<T>) -> Result<Self, Self::Error> {
        match value {
            ContractError::Panic(inner) => Ok(inner),
            _ => Err(value),
        }
    }
}

impl<T: fmt::Display> fmt::Display for ContractError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CustomError(error) => error.fmt(f),
            Self::Panic(panic) => panic.fmt(f),
            Self::Revert(revert) => revert.fmt(f),
        }
    }
}

impl<T: core::error::Error + 'static> core::error::Error for ContractError<T> {
    #[inline]
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::CustomError(error) => Some(error),
            Self::Panic(panic) => Some(panic),
            Self::Revert(revert) => Some(revert),
        }
    }
}

impl<T: SolInterface> SolInterface for ContractError<T> {
    const NAME: &'static str = "ContractError";

    // revert is 64, panic is 32
    const MIN_DATA_LENGTH: usize = if T::MIN_DATA_LENGTH < 32 { T::MIN_DATA_LENGTH } else { 32 };

    const COUNT: usize = T::COUNT + 2;

    #[inline]
    fn selector(&self) -> [u8; 4] {
        match self {
            Self::CustomError(error) => error.selector(),
            Self::Panic(_) => Panic::SELECTOR,
            Self::Revert(_) => Revert::SELECTOR,
        }
    }

    #[inline]
    fn selector_at(i: usize) -> Option<[u8; 4]> {
        if i < T::COUNT {
            T::selector_at(i)
        } else {
            match i - T::COUNT {
                0 => Some(Revert::SELECTOR),
                1 => Some(Panic::SELECTOR),
                _ => None,
            }
        }
    }

    #[inline]
    fn valid_selector(selector: [u8; 4]) -> bool {
        match selector {
            Revert::SELECTOR | Panic::SELECTOR => true,
            s => T::valid_selector(s),
        }
    }

    #[inline]
    fn abi_decode_raw(selector: [u8; 4], data: &[u8], validate: bool) -> Result<Self> {
        match selector {
            Revert::SELECTOR => Revert::abi_decode_raw(data, validate).map(Self::Revert),
            Panic::SELECTOR => Panic::abi_decode_raw(data, validate).map(Self::Panic),
            s => T::abi_decode_raw(s, data, validate).map(Self::CustomError),
        }
    }

    #[inline]
    fn abi_encoded_size(&self) -> usize {
        match self {
            Self::CustomError(error) => error.abi_encoded_size(),
            Self::Panic(panic) => panic.abi_encoded_size(),
            Self::Revert(revert) => revert.abi_encoded_size(),
        }
    }

    #[inline]
    fn abi_encode_raw(&self, out: &mut Vec<u8>) {
        match self {
            Self::CustomError(error) => error.abi_encode_raw(out),
            Self::Panic(panic) => panic.abi_encode_raw(out),
            Self::Revert(revert) => revert.abi_encode_raw(out),
        }
    }
}

impl<T> ContractError<T> {
    /// Returns `true` if `self` matches [`CustomError`](Self::CustomError).
    #[inline]
    pub const fn is_custom_error(&self) -> bool {
        matches!(self, Self::CustomError(_))
    }

    /// Returns an immutable reference to the inner custom error if `self`
    /// matches [`CustomError`](Self::CustomError).
    #[inline]
    pub const fn as_custom_error(&self) -> Option<&T> {
        match self {
            Self::CustomError(inner) => Some(inner),
            _ => None,
        }
    }

    /// Returns a mutable reference to the inner custom error if `self`
    /// matches [`CustomError`](Self::CustomError).
    #[inline]
    pub fn as_custom_error_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::CustomError(inner) => Some(inner),
            _ => None,
        }
    }

    /// Returns `true` if `self` matches [`Revert`](Self::Revert).
    #[inline]
    pub const fn is_revert(&self) -> bool {
        matches!(self, Self::Revert(_))
    }

    /// Returns an immutable reference to the inner [`Revert`] if `self` matches
    /// [`Revert`](Self::Revert).
    #[inline]
    pub const fn as_revert(&self) -> Option<&Revert> {
        match self {
            Self::Revert(inner) => Some(inner),
            _ => None,
        }
    }

    /// Returns a mutable reference to the inner [`Revert`] if `self` matches
    /// [`Revert`](Self::Revert).
    #[inline]
    pub fn as_revert_mut(&mut self) -> Option<&mut Revert> {
        match self {
            Self::Revert(inner) => Some(inner),
            _ => None,
        }
    }

    /// Returns `true` if `self` matches [`Panic`](Self::Panic).
    #[inline]
    pub const fn is_panic(&self) -> bool {
        matches!(self, Self::Panic(_))
    }

    /// Returns an immutable reference to the inner [`Panic`] if `self` matches
    /// [`Panic`](Self::Panic).
    #[inline]
    pub const fn as_panic(&self) -> Option<&Panic> {
        match self {
            Self::Panic(inner) => Some(inner),
            _ => None,
        }
    }

    /// Returns a mutable reference to the inner [`Panic`] if `self` matches
    /// [`Panic`](Self::Panic).
    #[inline]
    pub fn as_panic_mut(&mut self) -> Option<&mut Panic> {
        match self {
            Self::Panic(inner) => Some(inner),
            _ => None,
        }
    }
}

/// Represents the reason for a revert in a generic contract error.
pub type GenericRevertReason = RevertReason<Infallible>;

/// Represents the reason for a revert in a smart contract.
///
/// This enum captures two possible scenarios for a revert:
///
/// - [`ContractError`](RevertReason::ContractError): Contains detailed error information, such as a
///   specific [`Revert`] or [`Panic`] error.
///
/// - [`RawString`](RevertReason::RawString): Represents a raw string message as the reason for the
///   revert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevertReason<T> {
    /// A detailed contract error, including a specific revert or panic error.
    ContractError(ContractError<T>),
    /// Represents a raw string message as the reason for the revert.
    RawString(String),
}

impl<T: fmt::Display> fmt::Display for RevertReason<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContractError(error) => error.fmt(f),
            Self::RawString(raw_string) => f.write_str(raw_string),
        }
    }
}

/// Converts a `ContractError<T>` into a `RevertReason<T>`.
impl<T> From<ContractError<T>> for RevertReason<T> {
    fn from(error: ContractError<T>) -> Self {
        Self::ContractError(error)
    }
}

/// Converts a `Revert` into a `RevertReason<T>`.
impl<T> From<Revert> for RevertReason<T> {
    fn from(revert: Revert) -> Self {
        Self::ContractError(ContractError::Revert(revert))
    }
}

/// Converts a `String` into a `RevertReason<T>`.
impl<T> From<String> for RevertReason<T> {
    fn from(raw_string: String) -> Self {
        Self::RawString(raw_string)
    }
}

impl<T: SolInterface> RevertReason<T>
where
    Self: From<ContractError<Infallible>>,
{
    /// Decodes and retrieves the reason for a revert from the provided output data.
    ///
    /// This method attempts to decode the provided output data as a generic contract error
    /// or a UTF-8 string (for Vyper reverts).
    ///
    /// If successful, it returns the decoded revert reason wrapped in an `Option`.
    ///
    /// If both attempts fail, it returns `None`.
    pub fn decode(out: &[u8]) -> Option<Self> {
        // Try to decode as a generic contract error.
        if let Ok(error) = ContractError::<T>::abi_decode(out, false) {
            return Some(error.into());
        }

        // If that fails, try to decode as a regular string.
        if let Ok(decoded_string) = core::str::from_utf8(out) {
            return Some(decoded_string.to_string().into());
        }

        // If both attempts fail, return None.
        None
    }
}

impl<T: SolInterface + fmt::Display> RevertReason<T> {
    /// Returns the reason for a revert as a string.
    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        match self {
            Self::ContractError(error) => error.to_string(),
            Self::RawString(raw_string) => raw_string.clone(),
        }
    }
}

impl<T> RevertReason<T> {
    /// Returns the raw string error message if this type is a [`RevertReason::RawString`]
    pub fn as_raw_error(&self) -> Option<&str> {
        match self {
            Self::RawString(error) => Some(error.as_str()),
            _ => None,
        }
    }

    /// Returns the [`ContractError`] if this type is a [`RevertReason::ContractError`]
    pub const fn as_contract_error(&self) -> Option<&ContractError<T>> {
        match self {
            Self::ContractError(error) => Some(error),
            _ => None,
        }
    }

    /// Returns `true` if `self` matches [`Revert`](ContractError::Revert).
    pub const fn is_revert(&self) -> bool {
        matches!(self, Self::ContractError(ContractError::Revert(_)))
    }

    /// Returns `true` if `self` matches [`Panic`](ContractError::Panic).
    pub const fn is_panic(&self) -> bool {
        matches!(self, Self::ContractError(ContractError::Panic(_)))
    }

    /// Returns `true` if `self` matches [`CustomError`](ContractError::CustomError).
    pub const fn is_custom_error(&self) -> bool {
        matches!(self, Self::ContractError(ContractError::CustomError(_)))
    }
}

/// Iterator over the function or error selectors of a [`SolInterface`] type.
///
/// This `struct` is created by the [`selectors`] method on [`SolInterface`].
/// See its documentation for more.
///
/// [`selectors`]: SolInterface::selectors
pub struct Selectors<T> {
    index: usize,
    _marker: PhantomData<T>,
}

impl<T> Clone for Selectors<T> {
    fn clone(&self) -> Self {
        Self { index: self.index, _marker: PhantomData }
    }
}

impl<T> fmt::Debug for Selectors<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Selectors").field("index", &self.index).finish()
    }
}

impl<T> Selectors<T> {
    #[inline]
    const fn new() -> Self {
        Self { index: 0, _marker: PhantomData }
    }
}

impl<T: SolInterface> Iterator for Selectors<T> {
    type Item = [u8; 4];

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let selector = T::selector_at(self.index)?;
        self.index += 1;
        Some(selector)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let exact = self.len();
        (exact, Some(exact))
    }

    #[inline]
    fn count(self) -> usize {
        self.len()
    }
}

impl<T: SolInterface> ExactSizeIterator for Selectors<T> {
    #[inline]
    fn len(&self) -> usize {
        T::COUNT - self.index
    }
}

impl<T: SolInterface> FusedIterator for Selectors<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{keccak256, U256};

    fn sel(s: &str) -> [u8; 4] {
        keccak256(s)[..4].try_into().unwrap()
    }

    #[test]
    fn generic_contract_error_enum() {
        assert_eq!(
            GenericContractError::selectors().collect::<Vec<_>>(),
            [sel("Error(string)"), sel("Panic(uint256)")]
        );
    }

    #[test]
    fn contract_error_enum_1() {
        crate::sol! {
            contract C {
                error Err1();
            }
        }

        assert_eq!(C::CErrors::COUNT, 1);
        assert_eq!(C::CErrors::MIN_DATA_LENGTH, 0);
        assert_eq!(ContractError::<C::CErrors>::COUNT, 1 + 2);
        assert_eq!(ContractError::<C::CErrors>::MIN_DATA_LENGTH, 0);

        assert_eq!(C::CErrors::SELECTORS, [sel("Err1()")]);
        assert_eq!(
            ContractError::<C::CErrors>::selectors().collect::<Vec<_>>(),
            vec![sel("Err1()"), sel("Error(string)"), sel("Panic(uint256)")],
        );

        for selector in C::CErrors::selectors() {
            assert!(C::CErrors::valid_selector(selector));
        }

        for selector in ContractError::<C::CErrors>::selectors() {
            assert!(ContractError::<C::CErrors>::valid_selector(selector));
        }
    }

    #[test]
    fn contract_error_enum_2() {
        crate::sol! {
            #[derive(Debug, PartialEq, Eq)]
            contract C {
                error Err1();
                error Err2(uint256);
                error Err3(string);
            }
        }

        assert_eq!(C::CErrors::COUNT, 3);
        assert_eq!(C::CErrors::MIN_DATA_LENGTH, 0);
        assert_eq!(ContractError::<C::CErrors>::COUNT, 2 + 3);
        assert_eq!(ContractError::<C::CErrors>::MIN_DATA_LENGTH, 0);

        // sorted by selector
        assert_eq!(
            C::CErrors::SELECTORS,
            [sel("Err3(string)"), sel("Err2(uint256)"), sel("Err1()")]
        );
        assert_eq!(
            ContractError::<C::CErrors>::selectors().collect::<Vec<_>>(),
            [
                sel("Err3(string)"),
                sel("Err2(uint256)"),
                sel("Err1()"),
                sel("Error(string)"),
                sel("Panic(uint256)"),
            ],
        );

        let err1 = || C::Err1 {};
        let errors_err1 = || C::CErrors::Err1(err1());
        let contract_error_err1 = || ContractError::<C::CErrors>::CustomError(errors_err1());
        let data = err1().abi_encode();
        assert_eq!(data[..4], C::Err1::SELECTOR);
        assert_eq!(errors_err1().abi_encode(), data);
        assert_eq!(contract_error_err1().abi_encode(), data);

        assert_eq!(C::Err1::abi_decode(&data, true), Ok(err1()));
        assert_eq!(C::CErrors::abi_decode(&data, true), Ok(errors_err1()));
        assert_eq!(ContractError::<C::CErrors>::abi_decode(&data, true), Ok(contract_error_err1()));

        let err2 = || C::Err2 { _0: U256::from(42) };
        let errors_err2 = || C::CErrors::Err2(err2());
        let contract_error_err2 = || ContractError::<C::CErrors>::CustomError(errors_err2());
        let data = err2().abi_encode();
        assert_eq!(data[..4], C::Err2::SELECTOR);
        assert_eq!(errors_err2().abi_encode(), data);
        assert_eq!(contract_error_err2().abi_encode(), data);

        assert_eq!(C::Err2::abi_decode(&data, true), Ok(err2()));
        assert_eq!(C::CErrors::abi_decode(&data, true), Ok(errors_err2()));
        assert_eq!(ContractError::<C::CErrors>::abi_decode(&data, true), Ok(contract_error_err2()));

        let err3 = || C::Err3 { _0: "hello".into() };
        let errors_err3 = || C::CErrors::Err3(err3());
        let contract_error_err3 = || ContractError::<C::CErrors>::CustomError(errors_err3());
        let data = err3().abi_encode();
        assert_eq!(data[..4], C::Err3::SELECTOR);
        assert_eq!(errors_err3().abi_encode(), data);
        assert_eq!(contract_error_err3().abi_encode(), data);

        assert_eq!(C::Err3::abi_decode(&data, true), Ok(err3()));
        assert_eq!(C::CErrors::abi_decode(&data, true), Ok(errors_err3()));
        assert_eq!(ContractError::<C::CErrors>::abi_decode(&data, true), Ok(contract_error_err3()));

        for selector in C::CErrors::selectors() {
            assert!(C::CErrors::valid_selector(selector));
        }

        for selector in ContractError::<C::CErrors>::selectors() {
            assert!(ContractError::<C::CErrors>::valid_selector(selector));
        }
    }
}
