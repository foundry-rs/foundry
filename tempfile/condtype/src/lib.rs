//! [`CondType`]: CondType
//! [`condval!`]: condval
//! [`bool`]: bool
//! [`i32`]: i32
//! [`&str`]: str
//! [`rlim_t::MAX`]: u64::MAX
//! [Option]: Option
//! [None]: None
#![doc = include_str!("../README.md")]
#![cfg_attr(not(doc), no_std)]
#![warn(missing_docs)]

pub mod num;

/// [Conditionally aliases a type](crate#conditional-typing) using a [`bool`]
/// constant.
///
/// This is the Rust equivalent of [`std::conditional_t` in C++](https://en.cppreference.com/w/cpp/types/conditional).
/// Unlike the [`Either`] type, the type chosen by `CondType` is aliased, rather
/// than wrapped with an [`enum`] type. This may be considered a form of [dependent typing](https://en.wikipedia.org/wiki/Dependent_type),
/// but it is limited in ability and is restricted to compile-time constants
/// rather than runtime values.
///
/// [`enum`]: https://doc.rust-lang.org/std/keyword.enum.html
/// [`Either`]: https://docs.rs/either/latest/either/enum.Either.html
///
/// # Examples
///
/// In the following example, `CondType` aliases [`&str`](str) or [`i32`]:
///
/// ```
/// use condtype::CondType;
///
/// let str: CondType<true,  &str, i32> = "hello";
/// let int: CondType<false, &str, i32> = 42;
/// ```
///
/// This can also alias <code>\![Sized]</code> types:
///
/// ```
/// # use condtype::CondType;
/// type T = CondType<true, str, [u8]>;
///
/// let val: &T = "world";
/// ```
///
/// Rather than assign a value by knowing the condition, this can be used with
/// [`condval!`] to choose a value based on the same condition:
///
/// ```
/// # use condtype::*;
/// const COND: bool = // ...
/// # true;
///
/// let val: CondType<COND, &str, i32> = condval!(if COND {
///     "hello"
/// } else {
///     42
/// });
/// ```
pub type CondType<const B: bool, T, F> = <imp::CondType<B, T, F> as imp::AssocType>::Type;

/// Instantiates a [conditionally-typed](crate#conditional-typing) value.
///
/// Attempting to return different types from [`if`]/[`else`] is not normally
/// possible since both branches must produce the same type:
///
/// ```compile_fail
/// let val = if true { "hello" } else { 42 };
/// ```
///
/// This macro enables returning different types by making the type be
/// conditional on a [`const`] [`bool`]:
///
/// ```
/// use condtype::condval;
///
/// let val: &str = condval!(if true { "hello" } else { 42 });
/// let val: i32 = condval!(if false { "hello" } else { 42 });
/// ```
///
/// # Performance
///
/// This macro uses a pattern called ["TT munching"](https://veykril.github.io/tlborm/decl-macros/patterns/tt-muncher.html)
/// to parse the [`if`] condition expression. Compile times for TT munchers are
/// quadratic relative to the input length, so an expression like `!!!!!!!COND`
/// will compile slightly slower than `!COND` because it recurses 6 more times.
///
/// This can be mitigated by:
/// - Moving all logic in the [`if`] condition to a separate [`const`].
/// - Wrapping the logic in a block, e.g. `{ !true && !false }`.
///
/// # Examples
///
/// Given two conditions, the following code will construct either a
/// [`&str`](str), [`i32`], or [`Vec`]:
///
/// ```
/// # use condtype::condval;
/// const COND1: bool = // ...
/// # true;
/// const COND2: bool = // ...
/// # true;
///
/// let str = "hello";
/// let int = 42;
/// let vec = vec![1, 2, 3];
///
/// let val = condval!(if COND1 {
///     str
/// } else if COND2 {
///     int
/// } else {
///     vec
/// });
/// ```
///
/// `if let` pattern matching is also supported:
///
/// ```
/// # use condtype::*;
/// const STR: Option<&str> = // ...
/// # None;
///
/// let val = condval!(if let Some(str) = STR {
///     str.to_uppercase()
/// } else {
///     42
/// });
/// ```
///
/// This macro can be used with [`CondType`] to construct [`const`] values:
///
/// ```
/// use condtype::{condval, CondType};
///
/// const COND: bool = // ...
/// # true;
///
/// const VAL: CondType<COND, &str, i32> = condval!(if COND {
///     "hello"
/// } else {
///     42
/// });
/// ```
///
/// Each branch is lazily evaluated, so there are no effects from unvisited
/// branches:
///
/// ```
/// # use condtype::*;
/// let x;
///
/// let val = condval!(if true {
///     x = 10;
///     "hello"
/// } else {
///     x = 50;
///     42
/// });
///
/// assert_eq!(x, 10);
/// assert_eq!(val, "hello");
/// ```
///
/// Branch conditions can be any [`bool`] expression. However, see
/// [performance advice](#performance).
///
/// ```
/// # use condtype::*;
/// let val = condval!(if !true && !false {
///     "hello"
/// } else {
///     42
/// });
///
/// assert_eq!(val, 42);
/// ```
///
/// Assigning an incorrect type will cause a compile failure:
///
/// ```compile_fail
/// # use condtype::*;
/// let val: bool = condval!(if true {
///     "hello"
/// } else {
///     42
/// });
/// ```
///
/// Attempting to reuse a non-[`Copy`] value from either branch will cause a
/// compile failure, because it has been moved into that branch and can thus no
/// longer be used in the outer context:
///
/// ```compile_fail
/// # use condtype::*;
/// let int = 42;
/// let vec = vec![1, 2, 3];
///
/// let val = condval!(if true {
///     int
/// } else {
///     vec
/// });
///
/// println!("{:?}", vec);
/// ```
///
/// [`const`]: https://doc.rust-lang.org/std/keyword.const.html
/// [`else`]:  https://doc.rust-lang.org/std/keyword.else.html
/// [`if`]:    https://doc.rust-lang.org/std/keyword.if.html
#[macro_export]
macro_rules! condval {
    (if let $pat:pat = $input:block $then:block else $else:block) => {
        $crate::condval!(if {
            #[allow(unused_variables)]
            { ::core::matches!($input, $pat) }
        } {
            if let $pat = $input $then else {
                ::core::unreachable!()
            }
        } else $else)
    };
    (if let $pat:pat = $input:block $then:block else $($else:tt)+) => {
        $crate::condval!(if let $pat = $input $then else { $crate::condval!($($else)+) })
    };
    (if let $pat:pat = $($rest:tt)*) => {
        $crate::__condval_let_parser!($pat, [] $($rest)*)
    };
    (if $cond:block $then:block else $else:block) => {
        match <() as $crate::__private::If<$cond, _, _>>::PROOF {
            $crate::__private::EitherTypeEq::Left(te) => te.coerce($then),
            $crate::__private::EitherTypeEq::Right(te) => te.coerce($else),
        }
    };
    (if $cond:block $then:block else $($else:tt)+) => {
        $crate::condval!(if $cond $then else { $crate::condval!($($else)+) })
    };
    (if $($rest:tt)*) => {
        $crate::__condval_parser!([] $($rest)*)
    };
}

/// Helps `condval!` parse any `if` condition expression by accumulating tokens.
#[doc(hidden)]
#[macro_export]
macro_rules! __condval_parser {
    ([$($cond:tt)+] $then:block else $($else:tt)+) => {
        $crate::condval!(if { $($cond)+ } $then else $($else)+)
    };
    ([$($cond:tt)*] $next:tt $($rest:tt)*) => {
        $crate::__condval_parser!([$($cond)* $next] $($rest)*)
    };
}

/// Helps `condval!` parse any `if let` input expression by accumulating tokens.
#[doc(hidden)]
#[macro_export]
macro_rules! __condval_let_parser {
    ($pat:pat, [$($input:tt)+] $then:block else $($else:tt)+) => {
        $crate::condval!(if let $pat = { $($input)+ } $then else $($else)+)
    };
    ($pat:pat, [$($input:tt)*] $next:tt $($rest:tt)*) => {
        $crate::__condval_let_parser!($pat, [$($input)* $next] $($rest)*)
    };
}

/// Pseudo-public implementation details for `condval!`.
#[doc(hidden)]
pub mod __private {
    use crate::imp::TypeEq;

    pub enum EitherTypeEq<L, R, C> {
        Left(TypeEq<L, C>),
        Right(TypeEq<R, C>),
    }

    pub trait If<const B: bool, T, F> {
        type Chosen;
        const PROOF: EitherTypeEq<T, F, Self::Chosen>;
    }

    impl<T, F> If<true, T, F> for () {
        type Chosen = T;
        const PROOF: EitherTypeEq<T, F, Self::Chosen> = EitherTypeEq::Left(TypeEq::NEW);
    }

    impl<T, F> If<false, T, F> for () {
        type Chosen = F;
        const PROOF: EitherTypeEq<T, F, Self::Chosen> = EitherTypeEq::Right(TypeEq::NEW);
    }
}

/// Public-in-private implementation details.
mod imp {
    use core::{marker::PhantomData, mem::ManuallyDrop};

    pub struct CondType<const B: bool, T: ?Sized, F: ?Sized>(
        // `CondType` is covariant over `T` and `F`.
        PhantomData<F>,
        PhantomData<T>,
    );

    pub trait AssocType {
        type Type: ?Sized;
    }

    impl<T: ?Sized, F: ?Sized> AssocType for CondType<false, T, F> {
        type Type = F;
    }

    impl<T: ?Sized, F: ?Sized> AssocType for CondType<true, T, F> {
        type Type = T;
    }

    #[allow(clippy::type_complexity)]
    pub struct TypeEq<T, U>(
        PhantomData<(
            // `TypeEq` is invariant over `T` and `U`.
            fn(T) -> T,
            fn(U) -> U,
        )>,
    );

    impl<T> TypeEq<T, T> {
        pub const NEW: Self = TypeEq(PhantomData);
    }

    impl<T, U> TypeEq<T, U> {
        pub const fn coerce(self, from: T) -> U {
            #[repr(C)]
            union Transmuter<From, Into> {
                from: ManuallyDrop<From>,
                into: ManuallyDrop<Into>,
            }

            // SAFETY: `TypeEq` instances can only be constructed if `T` and `U`
            // are the same type.
            unsafe {
                ManuallyDrop::into_inner(
                    Transmuter {
                        from: ManuallyDrop::new(from),
                    }
                    .into,
                )
            }
        }
    }
}
