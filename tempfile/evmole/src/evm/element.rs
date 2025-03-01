use crate::evm::U256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Element<T> {
    pub data: [u8; 32],
    pub label: Option<T>,
}

macro_rules! impl_tryfrom_element {
    ($t:ty) => {
        impl<T> TryFrom<Element<T>> for $t {
            type Error = alloy_primitives::ruint::FromUintError<$t>;

            fn try_from(val: Element<T>) -> Result<Self, Self::Error> {
                U256::from_be_bytes(val.data).try_into()
            }
        }

        impl<T> TryFrom<&Element<T>> for $t {
            type Error = alloy_primitives::ruint::FromUintError<$t>;

            fn try_from(val: &Element<T>) -> Result<Self, Self::Error> {
                U256::from_be_bytes(val.data).try_into()
            }
        }
    };
}
impl_tryfrom_element!(u8);
impl_tryfrom_element!(u32);
impl_tryfrom_element!(usize);

impl<T> From<Element<T>> for U256 {
    fn from(val: Element<T>) -> Self {
        U256::from_be_bytes(val.data)
    }
}

impl<T> From<&Element<T>> for U256 {
    fn from(val: &Element<T>) -> Self {
        U256::from_be_bytes(val.data)
    }
}
