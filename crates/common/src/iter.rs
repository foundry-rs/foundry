use std::iter::Peekable;

pub struct Delimited<I: Iterator> {
    is_first: bool,
    iter: Peekable<I>,
}

pub trait IterDelimited: Iterator + Sized {
    fn delimited(self) -> Delimited<Self> {
        Delimited { is_first: true, iter: self.peekable() }
    }
}

impl<I: Iterator> IterDelimited for I {}

pub struct IteratorPosition {
    pub is_first: bool,
    pub is_last: bool,
}

impl<I: Iterator> Iterator for Delimited<I> {
    type Item = (IteratorPosition, I::Item);

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.iter.next()?;
        let position =
            IteratorPosition { is_first: self.is_first, is_last: self.iter.peek().is_none() };
        self.is_first = false;
        Some((position, item))
    }
}
