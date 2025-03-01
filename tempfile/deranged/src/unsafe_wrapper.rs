#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Unsafe<T>(T);

impl<T: core::fmt::Debug> core::fmt::Debug for Unsafe<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> Unsafe<T> {
    pub(crate) const unsafe fn new(value: T) -> Self {
        Self(value)
    }

    pub(crate) const fn get(&self) -> &T {
        &self.0
    }
}

impl<T> core::ops::Deref for Unsafe<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
