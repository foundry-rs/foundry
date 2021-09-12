pub mod opts;

mod utils;

mod seth;
pub use seth::*;

// Re-export Ethers for convenience.
pub use ethers;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
