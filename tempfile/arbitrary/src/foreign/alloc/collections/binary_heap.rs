use {
    crate::{Arbitrary, Result, Unstructured},
    std::collections::binary_heap::BinaryHeap,
};

impl<'a, A> Arbitrary<'a> for BinaryHeap<A>
where
    A: Arbitrary<'a> + Ord,
{
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        u.arbitrary_iter()?.collect()
    }

    fn arbitrary_take_rest(u: Unstructured<'a>) -> Result<Self> {
        u.arbitrary_take_rest_iter()?.collect()
    }

    #[inline]
    fn size_hint(_depth: usize) -> (usize, Option<usize>) {
        (0, None)
    }
}
