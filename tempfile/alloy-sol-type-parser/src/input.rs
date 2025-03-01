#![allow(missing_docs, missing_copy_implementations, missing_debug_implementations)]

// Recursion implementation modified from `toml`: https://github.com/toml-rs/toml/blob/a02cbf46cab4a8683e641efdba648a31498f7342/crates/toml_edit/src/parser/mod.rs#L99

use core::fmt;
use winnow::{error::ContextError, ModalParser};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomError {
    RecursionLimitExceeded,
}

impl fmt::Display for CustomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecursionLimitExceeded => f.write_str("recursion limit exceeded"),
        }
    }
}

impl core::error::Error for CustomError {}

pub type Input<'a> = winnow::Stateful<&'a str, RecursionCheck>;

#[inline]
pub fn new_input(input: &str) -> Input<'_> {
    winnow::Stateful { input, state: Default::default() }
}

pub fn check_recursion<'a, O>(
    mut parser: impl ModalParser<Input<'a>, O, ContextError>,
) -> impl ModalParser<Input<'a>, O, ContextError> {
    move |input: &mut Input<'a>| {
        input.state.enter().map_err(|_err| {
            // TODO: Very weird bug with features: https://github.com/alloy-rs/core/issues/717
            // use winnow::error::FromExternalError;
            // let err = winnow::error::ContextError::from_external_error(input, _err);
            let err = winnow::error::ContextError::new();
            winnow::error::ErrMode::Cut(err)
        })?;
        let result = parser.parse_next(input);
        input.state.exit();
        result
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecursionCheck {
    current: usize,
}

const LIMIT: usize = 80;

impl RecursionCheck {
    #[cfg(any())]
    fn check_depth(_depth: usize) -> Result<(), CustomError> {
        if LIMIT <= _depth {
            return Err(CustomError::RecursionLimitExceeded);
        }

        Ok(())
    }

    fn enter(&mut self) -> Result<(), CustomError> {
        self.current += 1;
        if LIMIT <= self.current {
            return Err(CustomError::RecursionLimitExceeded);
        }
        Ok(())
    }

    fn exit(&mut self) {
        self.current -= 1;
    }
}
