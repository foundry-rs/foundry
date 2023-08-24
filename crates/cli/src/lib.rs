#![warn(unused_crate_dependencies)]

pub mod handler;
pub mod opts;
pub mod utils;

mod io;
pub use io::{shell, stdin};
