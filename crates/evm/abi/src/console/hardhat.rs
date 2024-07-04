use alloy_primitives::Selector;
use alloy_sol_types::sol;
use foundry_common_fmt::*;
use foundry_macros::ConsoleFmt;
use once_cell::sync::Lazy;
use rustc_hash::FxHashMap;

sol!(
    #[sol(abi)]
    #[derive(ConsoleFmt)]
    HardhatConsole,
    "src/HardhatConsole.json"
);

/// Patches the given Hardhat `console` function selector to its ABI-normalized form.
///
/// See [`HARDHAT_CONSOLE_SELECTOR_PATCHES`] for more details.
pub fn patch_hh_console_selector(input: &mut [u8]) {
    if let Some(selector) = hh_console_selector(input) {
        input[..4].copy_from_slice(selector.as_slice());
    }
}

/// Returns the ABI-normalized selector for the given Hardhat `console` function selector.
///
/// See [`HARDHAT_CONSOLE_SELECTOR_PATCHES`] for more details.
pub fn hh_console_selector(input: &[u8]) -> Option<&'static Selector> {
    if let Some(selector) = input.get(..4) {
        let selector: &[u8; 4] = selector.try_into().unwrap();
        HARDHAT_CONSOLE_SELECTOR_PATCHES.get(selector).map(Into::into)
    } else {
        None
    }
}

/// Maps all the `hardhat/console.log` log selectors that use the legacy ABI (`int`, `uint`) to
/// their normalized counterparts (`int256`, `uint256`).
///
/// `hardhat/console.log` logs its events manually, and in functions that accept integers they're
/// encoded as `abi.encodeWithSignature("log(int)", p0)`, which is not the canonical ABI encoding
/// for `int` that Solidity and [`sol!`] use.
pub static HARDHAT_CONSOLE_SELECTOR_PATCHES: Lazy<FxHashMap<[u8; 4], [u8; 4]>> =
    Lazy::new(|| FxHashMap::from_iter(include!("./patches.rs")));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardhat_console_patch() {
        for (hh, generated) in HARDHAT_CONSOLE_SELECTOR_PATCHES.iter() {
            let mut hh = *hh;
            patch_hh_console_selector(&mut hh);
            assert_eq!(hh, *generated);
        }
    }
}
