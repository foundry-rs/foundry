use itertools::Itertools;
use solang_parser::pt::*;

/// Describes the default sort of attributes
pub trait AttrSortKey {
    fn attr_sort_key(&self) -> usize;
}

impl AttrSortKey for VariableAttribute {
    fn attr_sort_key(&self) -> usize {
        match self {
            VariableAttribute::Visibility(..) => 0,
            VariableAttribute::Constant(..) => 1,
            VariableAttribute::Immutable(..) => 2,
            VariableAttribute::Override(..) => 3,
        }
    }
}

impl AttrSortKey for FunctionAttribute {
    fn attr_sort_key(&self) -> usize {
        match self {
            FunctionAttribute::Visibility(..) => 0,
            FunctionAttribute::Mutability(..) => 1,
            FunctionAttribute::Virtual(..) => 2,
            FunctionAttribute::Immutable(..) => 3,
            FunctionAttribute::Override(..) => 4,
            FunctionAttribute::BaseOrModifier(..) => 5,
            FunctionAttribute::Error(..) => 6, // supposed to be omitted even if sorted
        }
    }
}

impl<T> AttrSortKey for &T
where
    T: AttrSortKey,
{
    fn attr_sort_key(&self) -> usize {
        T::attr_sort_key(self)
    }
}

impl<T> AttrSortKey for &mut T
where
    T: AttrSortKey,
{
    fn attr_sort_key(&self) -> usize {
        T::attr_sort_key(self)
    }
}

pub trait AttrSortKeyIteratorExt: Iterator {
    fn attr_sorted(self) -> std::vec::IntoIter<Self::Item>;
}

impl<I> AttrSortKeyIteratorExt for I
where
    I: Iterator,
    I::Item: AttrSortKey,
{
    fn attr_sorted(self) -> std::vec::IntoIter<Self::Item> {
        self.sorted_by_key(|item| item.attr_sort_key())
    }
}
