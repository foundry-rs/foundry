use std::{
    any::{Any, TypeId},
    marker::PhantomData,
};

/// Returns a [`TypeId`] for any type regardless of whether it is `'static`.
///
/// Note that **this is not the same** as [`TypeId::of`].
#[inline]
pub(crate) fn proxy_type_id<T: ?Sized>() -> TypeId {
    // Return the type ID of a generic closure.
    Any::type_id(&|| PhantomData::<T>)
}

/// Returns `true` if the given types are equal.
#[inline]
pub(crate) fn is_type_eq<A: ?Sized, B: ?Sized>() -> bool {
    proxy_type_id::<A>() == proxy_type_id::<B>()
}

/// Convenience trait for type conversions.
pub(crate) trait TypeCast {
    /// Converts a reference if `self` is an instance of `T`.
    ///
    /// We require `T: 'static` since we want to ensure when providing a type
    /// that any lifetimes are static, such as `Cow<str>`.
    #[inline]
    fn cast_ref<T: 'static>(&self) -> Option<&T> {
        if is_type_eq::<Self, T>() {
            // SAFETY: `self` is `&T`.
            Some(unsafe { &*(self as *const Self as *const T) })
        } else {
            None
        }
    }
}

impl<A> TypeCast for A {}
