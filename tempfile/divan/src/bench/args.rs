//! Types used to implement runtime argument support.

use std::{
    any::{Any, TypeId},
    borrow::Cow,
    mem, slice,
    sync::OnceLock,
};

use crate::{util::ty::TypeCast, Bencher};

/// Holds lazily-initialized runtime arguments to be passed into a benchmark.
///
/// `#[divan::bench]` stores this as a `__DIVAN_ARGS` global for each entry, and
/// then at runtime it is initialized once by a closure that creates the usable
/// `BenchArgsRunner`.
pub struct BenchArgs {
    args: OnceLock<ErasedArgsSlice>,
}

/// The result of making `BenchArgs` runnable from instantiating the arguments
/// list and providing a typed benchmarking implementation.
#[derive(Clone, Copy)]
pub struct BenchArgsRunner {
    args: &'static ErasedArgsSlice,
    bench: fn(Bencher, &ErasedArgsSlice, arg_index: usize),
}

/// Type-erased `&'static [T]` that also stores names of the arguments.
struct ErasedArgsSlice {
    /// The start of `&[T]`.
    args: *const (),

    /// The start of `&[&'static str]`.
    names: *const &'static str,

    /// The number of arguments.
    len: usize,

    /// The ID of `T` to ensure correctness.
    arg_type: TypeId,
}

// SAFETY: Raw pointers in `ErasedArgsSlice` are used in a thread-safe way, and
// the argument type is required to be `Send + Sync` when initialized from the
// iterator in `BenchArgs::runner`.
unsafe impl Send for ErasedArgsSlice {}
unsafe impl Sync for ErasedArgsSlice {}

impl BenchArgs {
    /// Creates an uninitialized instance.
    pub const fn new() -> Self {
        Self { args: OnceLock::new() }
    }

    /// Initializes `self` with the results of `make_args` and returns a
    /// `BenchArgsRunner` that will execute the benchmarking closure.
    pub fn runner<I, B>(
        &'static self,
        make_args: impl FnOnce() -> I,
        arg_to_string: impl Fn(&I::Item) -> String,
        _bench_impl: B,
    ) -> BenchArgsRunner
    where
        I: IntoIterator,
        I::Item: Any + Send + Sync,
        B: FnOnce(Bencher, &I::Item) + Copy,
    {
        let args = self.args.get_or_init(|| {
            let args_iter = make_args().into_iter();

            // Reuse arguments for names if already a slice of strings.
            //
            // NOTE: We do this over `I::IntoIter` instead of `I` since it works
            // for both slices and `slice::Iter`.
            let args_strings: Option<&'static [&str]> =
                args_iter.cast_ref::<slice::Iter<&str>>().map(|iter| iter.as_slice());

            // Collect arguments into leaked slice.
            //
            // Leaking the collected `args` simplifies memory management, such
            // as when reusing for `names`. We're leaking anyways since this is
            // accessed via a global `OnceLock`.
            //
            // PERF: We could optimize this to reuse arguments when users
            // provide slices. However, for slices its `Item` is a reference, so
            // `slice::Iter<I::Item>` would never match here. To make this
            // optimization, we would need to be able to get the referee type.
            let args: &'static [I::Item] = Box::leak(args_iter.collect());

            // Collect printable representations of arguments.
            //
            // PERF: We take multiple opportunities to reuse the provided
            // arguments buffer or individual strings' buffers:
            // - `&[&str]`
            // - `IntoIterator<Item = &str>`
            // - `IntoIterator<Item = String>`
            // - `IntoIterator<Item = Box<str>>`
            // - `IntoIterator<Item = Cow<str>>`
            let names: &'static [&str] = 'names: {
                // PERF: Reuse arguments strings slice.
                if let Some(args) = args_strings {
                    break 'names args;
                }

                // PERF: Reuse our args slice allocation.
                if let Some(args) = args.cast_ref::<&[&str]>() {
                    break 'names args;
                }

                Box::leak(
                    args.iter()
                        .map(|arg| -> &str {
                            // PERF: Reuse strings as-is.
                            if let Some(arg) = arg.cast_ref::<String>() {
                                return arg;
                            }
                            if let Some(arg) = arg.cast_ref::<Box<str>>() {
                                return arg;
                            }
                            if let Some(arg) = arg.cast_ref::<Cow<str>>() {
                                return arg;
                            }

                            // Default to `arg_to_string`, which will format via
                            // either `ToString` or `Debug`.
                            Box::leak(arg_to_string(arg).into_boxed_str())
                        })
                        .collect(),
                )
            };

            ErasedArgsSlice {
                // We `black_box` arguments to prevent the compiler from
                // optimizing the benchmark for the provided values.
                args: crate::black_box(args.as_ptr().cast()),
                names: names.as_ptr(),
                len: args.len(),
                arg_type: TypeId::of::<I::Item>(),
            }
        });

        BenchArgsRunner { args, bench: bench::<I::Item, B> }
    }
}

impl Default for BenchArgs {
    fn default() -> Self {
        Self::new()
    }
}

impl BenchArgsRunner {
    #[inline]
    pub(crate) fn bench(&self, bencher: Bencher, index: usize) {
        (self.bench)(bencher, self.args, index)
    }

    #[inline]
    pub(crate) fn arg_names(&self) -> &'static [&'static str] {
        self.args.names()
    }
}

impl ErasedArgsSlice {
    /// Retrieves a slice of arguments if the type is `T`.
    #[inline]
    fn typed_args<T: Any>(&self) -> Option<&[T]> {
        if self.arg_type == TypeId::of::<T>() {
            // SAFETY: `BenchArgs::runner` guarantees storing `len` instances.
            Some(unsafe { slice::from_raw_parts(self.args.cast(), self.len) })
        } else {
            None
        }
    }

    /// Returns the arguments' names.
    ///
    /// Names are in the same order as args and thus their indices can be used
    /// to reference arguments.
    #[inline]
    fn names(&self) -> &'static [&str] {
        // SAFETY: `BenchArgs::runner` guarantees storing `len` names.
        unsafe { slice::from_raw_parts(self.names, self.len) }
    }
}

/// The `BenchArgsRunner.bench` implementation.
fn bench<T, B>(bencher: Bencher, erased_args: &ErasedArgsSlice, arg_index: usize)
where
    T: Any,
    B: FnOnce(Bencher, &T) + Copy,
{
    // We defer type checking until the benchmark is run to make safety of this
    // function easier to audit. Checking here instead of in `BenchArgs::runner`
    // is late but fine since this check will only fail due to a bug in Divan's
    // macro code generation.

    let Some(typed_args) = erased_args.typed_args::<T>() else {
        type_mismatch::<T>();

        // Reduce code size by using a separate function for each `T` instead of
        // each benchmark closure.
        #[cold]
        #[inline(never)]
        fn type_mismatch<T>() -> ! {
            unreachable!("incorrect type '{}'", std::any::type_name::<T>())
        }
    };

    // SAFETY: The closure is a ZST, so we can construct one out of thin air.
    // This can be done multiple times without invoking a `Drop` destructor
    // because it implements `Copy`.
    let bench_impl: B = unsafe {
        assert_eq!(size_of::<B>(), 0, "benchmark closure expected to be zero-sized");
        mem::zeroed()
    };

    bench_impl(bencher, &typed_args[arg_index]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that optimizations for string items are applied.
    mod optimizations {
        use std::borrow::Borrow;

        use super::*;

        /// Tests that two slices contain the same exact strings.
        fn test_eq_ptr<A: Borrow<str>, B: Borrow<str>>(a: &[A], b: &[B]) {
            assert_eq!(a.len(), b.len());

            for (a, b) in a.iter().zip(b) {
                let a = a.borrow();
                let b = b.borrow();
                assert_eq!(a, b);
                assert_eq!(a.as_ptr(), b.as_ptr());
            }
        }

        /// Tests that `&[&str]` reuses the original slice for names.
        #[test]
        fn str_slice() {
            static ARGS: BenchArgs = BenchArgs::new();
            static ORIG_ARGS: &[&str] = &["a", "b"];

            let runner = ARGS.runner(|| ORIG_ARGS, ToString::to_string, |_, _| {});

            let typed_args: Vec<&str> =
                runner.args.typed_args::<&&str>().unwrap().iter().copied().copied().collect();
            let names = runner.arg_names();

            // Test values.
            assert_eq!(names, ORIG_ARGS);
            assert_eq!(names, typed_args);

            // Test addresses.
            assert_eq!(names.as_ptr(), ORIG_ARGS.as_ptr());
            assert_ne!(names.as_ptr(), typed_args.as_ptr());
        }

        /// Tests optimizing `IntoIterator<Item = &str>` to reuse the same
        /// allocation for also storing argument names.
        #[test]
        fn str_array() {
            static ARGS: BenchArgs = BenchArgs::new();

            let runner = ARGS.runner(|| ["a", "b"], ToString::to_string, |_, _| {});

            let typed_args = runner.args.typed_args::<&str>().unwrap();
            let names = runner.arg_names();

            // Test values.
            assert_eq!(names, ["a", "b"]);
            assert_eq!(names, typed_args);

            // Test addresses.
            assert_eq!(names.as_ptr(), typed_args.as_ptr());
        }

        /// Tests optimizing `IntoIterator<Item = String>` to reuse the same
        /// allocation for also storing argument names.
        #[test]
        fn string_array() {
            static ARGS: BenchArgs = BenchArgs::new();

            let runner =
                ARGS.runner(|| ["a".to_owned(), "b".to_owned()], ToString::to_string, |_, _| {});

            let typed_args = runner.args.typed_args::<String>().unwrap();
            let names = runner.arg_names();

            assert_eq!(names, ["a", "b"]);
            test_eq_ptr(names, typed_args);
        }

        /// Tests optimizing `IntoIterator<Item = Box<str>>` to reuse the same
        /// allocation for also storing argument names.
        #[test]
        fn box_str_array() {
            static ARGS: BenchArgs = BenchArgs::new();

            let runner = ARGS.runner(
                || ["a".to_owned().into_boxed_str(), "b".to_owned().into_boxed_str()],
                ToString::to_string,
                |_, _| {},
            );

            let typed_args = runner.args.typed_args::<Box<str>>().unwrap();
            let names = runner.arg_names();

            assert_eq!(names, ["a", "b"]);
            test_eq_ptr(names, typed_args);
        }

        /// Tests optimizing `IntoIterator<Item = Cow<str>>` to reuse the same
        /// allocation for also storing argument names.
        #[test]
        fn cow_str_array() {
            static ARGS: BenchArgs = BenchArgs::new();

            let runner = ARGS.runner(
                || [Cow::Owned("a".to_owned()), Cow::Borrowed("b")],
                ToString::to_string,
                |_, _| {},
            );

            let typed_args = runner.args.typed_args::<Cow<str>>().unwrap();
            let names = runner.arg_names();

            assert_eq!(names, ["a", "b"]);
            test_eq_ptr(names, typed_args);
        }
    }
}
