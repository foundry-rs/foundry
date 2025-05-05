use std::{
    collections::VecDeque,
    ops::{Index, IndexMut, Range},
};

#[derive(Debug)]
pub(crate) struct RingBuffer<T> {
    data: VecDeque<T>,
    // Abstract index of data[0] in the infinitely sized queue.
    offset: usize,
}

impl<T> RingBuffer<T> {
    pub(crate) fn new() -> Self {
        Self { data: VecDeque::new(), offset: 0 }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len()
    }

    pub(crate) fn push(&mut self, value: T) -> usize {
        let index = self.offset + self.data.len();
        self.data.push_back(value);
        index
    }

    pub(crate) fn clear(&mut self) {
        self.data.clear();
    }

    pub(crate) fn index_range(&self) -> Range<usize> {
        self.offset..self.offset + self.data.len()
    }

    #[inline]
    #[track_caller]
    pub(crate) fn first(&self) -> &T {
        &self.data[0]
    }

    #[inline]
    #[track_caller]
    pub(crate) fn first_mut(&mut self) -> &mut T {
        &mut self.data[0]
    }

    #[inline]
    #[track_caller]
    pub(crate) fn pop_first(&mut self) -> T {
        self.offset += 1;
        self.data.pop_front().unwrap()
    }

    #[inline]
    #[track_caller]
    pub(crate) fn last(&self) -> &T {
        self.data.back().unwrap()
    }

    #[inline]
    #[track_caller]
    pub(crate) fn last_mut(&mut self) -> &mut T {
        self.data.back_mut().unwrap()
    }

    #[inline]
    #[track_caller]
    pub(crate) fn second_last(&self) -> &T {
        &self.data[self.data.len() - 2]
    }

    #[inline]
    #[track_caller]
    pub(crate) fn pop_last(&mut self) {
        self.data.pop_back().unwrap();
    }
}

impl<T> Index<usize> for RingBuffer<T> {
    type Output = T;
    fn index(&self, index: usize) -> &Self::Output {
        &self.data[index.checked_sub(self.offset).unwrap()]
    }
}

impl<T> IndexMut<usize> for RingBuffer<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.data[index.checked_sub(self.offset).unwrap()]
    }
}
