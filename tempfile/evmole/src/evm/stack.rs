use super::{element::Element, U256};
use std::fmt;

#[derive(Clone)]
pub struct Stack<T> {
    pub data: Vec<Element<T>>,
}

impl<T: fmt::Debug> fmt::Debug for Stack<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} elems:", self.data.len())?;
        for el in &self.data {
            write!(
                f,
                "\n  - {} | {:?}",
                el.data
                    .iter()
                    .map(|x| format!("{:02x}", x))
                    .collect::<Vec<_>>()
                    .join(""),
                el.label
            )?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct IndexError;

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "IndexError")
    }
}

impl std::error::Error for IndexError {}

type Result<T> = std::result::Result<T, IndexError>;

impl<T: Clone> Stack<T> {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn push(&mut self, val: Element<T>) {
        self.data.push(val)
    }

    pub fn pop(&mut self) -> Result<Element<T>> {
        self.data.pop().ok_or(IndexError)
    }

    pub fn peek(&self) -> Result<&Element<T>> {
        self.data.last().ok_or(IndexError)
    }

    pub fn peek_mut(&mut self) -> Result<&mut Element<T>> {
        self.data.last_mut().ok_or(IndexError)
    }

    pub fn dup(&mut self, n: u8) -> Result<()> {
        let idx = n as usize;
        if self.data.len() < idx {
            Err(IndexError)
        } else {
            self.data.push(self.data[self.data.len() - idx].clone());
            Ok(())
        }
    }

    pub fn swap(&mut self, n: u8) -> Result<()> {
        let dlen = self.data.len();
        let idx = n as usize;
        if dlen <= idx {
            Err(IndexError)
        } else {
            self.data.swap(dlen - 1, dlen - 1 - idx);
            Ok(())
        }
    }

    pub fn push_data(&mut self, data: [u8; 32]) {
        self.data.push(Element { data, label: None });
    }

    pub fn push_uint(&mut self, val: U256) {
        self.push_data(val.to_be_bytes());
    }

    pub fn pop_uint(&mut self) -> Result<U256> {
        match self.data.pop() {
            Some(v) => Ok(v.into()),
            None => Err(IndexError),
        }
    }
}
