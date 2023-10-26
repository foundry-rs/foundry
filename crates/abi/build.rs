//! **WARNING**
//! This crate is deprecated and will be replaced with `foundry-cheatcodes` in the near future.
//! Please avoid making any changes in this crate.

use ethers_contract_abigen::MultiAbigen;

/// Includes a JSON ABI as a string literal.
macro_rules! include_json_abi {
    ($path:literal) => {{
        println!(concat!("cargo:rerun-if-changed=", $path));
        include_str!($path)
    }};
}

/// Includes a human-readable ABI file as a string literal by wrapping it in brackets.
macro_rules! include_hr_abi {
    ($path:literal) => {{
        println!(concat!("cargo:rerun-if-changed=", $path));
        concat!("[\n", include_str!($path), "\n]")
    }};
}

fn main() -> eyre::Result<()> {
    let mut multi = MultiAbigen::new([
        ("HardhatConsole", include_json_abi!("abi/HardhatConsole.json")),
        ("Console", include_hr_abi!("abi/Console.sol")),
        ("HEVM", include_hr_abi!("abi/HEVM.sol")),
    ])?;

    // Add the ConsoleFmt derive to the HardhatConsole contract
    multi[0].derives_mut().push(syn::parse_str("foundry_macros::ConsoleFmt")?);

    // Generate and write to the bindings module
    let bindings = multi.build()?;
    bindings.write_to_module("src/bindings/", false)?;

    Ok(())
}
