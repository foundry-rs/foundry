use std::{
    cell::UnsafeCell,
    mem::{ManuallyDrop, MaybeUninit},
};

/// Defers input usage and output drop during benchmarking.
///
/// To reduce memory usage, this only allocates storage for inputs if outputs do
/// not need deferred drop.
pub(crate) union DeferStore<I, O> {
    /// The variant used if outputs need to be dropped.
    ///
    /// Inputs are stored are stored contiguously with outputs in memory. This
    /// improves performance by:
    /// - Removing the overhead of `zip` between two separate buffers.
    /// - Improving cache locality and cache prefetching. Input is strategically
    ///   placed before output because iteration is from low to high addresses,
    ///   so doing this makes memory access patterns very predictable.
    slots: ManuallyDrop<Vec<DeferSlot<I, O>>>,

    /// The variant used if `Self::ONLY_INPUTS`, i.e. outputs do not need to be
    /// dropped.
    inputs: ManuallyDrop<Vec<DeferSlotItem<I>>>,
}

impl<I, O> Drop for DeferStore<I, O> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: The correct variant is used based on `ONLY_INPUTS`.
        unsafe {
            if Self::ONLY_INPUTS {
                ManuallyDrop::drop(&mut self.inputs)
            } else {
                ManuallyDrop::drop(&mut self.slots)
            }
        }
    }
}

impl<I, O> Default for DeferStore<I, O> {
    #[inline]
    fn default() -> Self {
        // SAFETY: The correct variant is used based on `ONLY_INPUTS`.
        unsafe {
            if Self::ONLY_INPUTS {
                Self { inputs: ManuallyDrop::new(Vec::new()) }
            } else {
                Self { slots: ManuallyDrop::new(Vec::new()) }
            }
        }
    }
}

impl<I, O> DeferStore<I, O> {
    /// Whether only inputs need to be deferred.
    ///
    /// If `true`, outputs do not get inserted into `DeferStore`.
    const ONLY_INPUTS: bool = !std::mem::needs_drop::<O>();

    /// Prepares storage for iterating over `DeferSlot`s for a sample.
    #[inline]
    pub fn prepare(&mut self, sample_size: usize) {
        // Common implementation regardless of `Vec` item type.
        macro_rules! imp {
            ($vec:expr) => {{
                $vec.clear();
                $vec.reserve_exact(sample_size);

                // SAFETY: `Vec` only contains `MaybeUninit` fields, so values
                // may be safely created from uninitialized memory.
                unsafe { $vec.set_len(sample_size) }
            }};
        }

        // SAFETY: The correct variant is used based on `ONLY_INPUTS`.
        unsafe {
            if Self::ONLY_INPUTS {
                imp!(self.inputs)
            } else {
                imp!(self.slots)
            }
        }
    }

    /// Returns the sample's slots for iteration.
    ///
    /// The caller is expected to use the returned slice to initialize inputs
    /// for the sample loop.
    ///
    /// This returns `Err` containing only input slots if `O` does not need
    /// deferred drop. Ideally this would be implemented directly on `DeferSlot`
    /// but there's no way to change its size based on `needs_drop::<O>()`.
    #[inline(always)]
    pub fn slots(&self) -> Result<&[DeferSlot<I, O>], &[DeferSlotItem<I>]> {
        unsafe {
            if Self::ONLY_INPUTS {
                Err(&self.inputs)
            } else {
                Ok(&self.slots)
            }
        }
    }
}

/// Storage for a single iteration within a sample.
///
/// Input is stored before output to improve cache prefetching since iteration
/// progresses from low to high addresses.
///
/// # UnsafeCell
///
/// `UnsafeCell` is used to allow `output` to safely refer to `input`. Although
/// `output` itself is never aliased, it is also stored as `UnsafeCell` in order
/// to get mutable access through a shared `&DeferSlot`.
///
/// # Safety
///
/// All fields **must** be `MaybeUninit`. This allows us to safely set the
/// length of `Vec<DeferSlot>` within the allocated capacity.
#[repr(C)]
pub(crate) struct DeferSlot<I, O> {
    pub input: DeferSlotItem<I>,
    pub output: DeferSlotItem<O>,
}

type DeferSlotItem<T> = UnsafeCell<MaybeUninit<T>>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that accessing an uninitialized `DeferSlot` is safe due to all of
    /// its fields being `MaybeUninit`.
    #[test]
    fn access_uninit_slot() {
        let mut slot: MaybeUninit<DeferSlot<String, String>> = MaybeUninit::uninit();

        let slot_ref = unsafe { slot.assume_init_mut() };
        slot_ref.input = UnsafeCell::new(MaybeUninit::new(String::new()));
        slot_ref.output = UnsafeCell::new(MaybeUninit::new(String::new()));

        unsafe {
            let slot = slot.assume_init();
            assert_eq!(slot.input.into_inner().assume_init(), "");
            assert_eq!(slot.output.into_inner().assume_init(), "");
        }
    }

    /// Tests that accessing `DeferSlot.input` through an aliased reference in
    /// `DeferSlot.output` is safe due `input` being an `UnsafeCell`.
    #[test]
    fn access_aliased_input() {
        struct Output<'i> {
            input: &'i mut String,
        }

        impl Drop for Output<'_> {
            fn drop(&mut self) {
                assert_eq!(self.input, "hello");
                self.input.push_str(" world");
            }
        }

        let slot: MaybeUninit<DeferSlot<String, Output>> = MaybeUninit::uninit();
        let slot_ref = unsafe { slot.assume_init_ref() };

        // Loop to ensure previous iterations don't affect later uses of the
        // same entry slot.
        for _ in 0..5 {
            unsafe {
                let input_ptr = slot_ref.input.get().cast::<String>();
                let output_ptr = slot_ref.output.get().cast::<Output>();

                // Initialize input and output.
                input_ptr.write("hello".to_owned());
                output_ptr.write(Output { input: &mut *input_ptr });

                // Use and discard output.
                assert_eq!((*output_ptr).input, "hello");
                output_ptr.drop_in_place();
                assert_eq!(&*input_ptr, "hello world");

                // Discard input.
                input_ptr.drop_in_place();
            }
        }
    }
}
