//! The `anvil-polkadot` CLI: a fast local ethereum-compatible development node, based on
//! polkadot-sdk.

fn main() {
    if let Err(err) = anvil_polkadot::run() {
        let _ = foundry_common::sh_err!("{err:?}");
        std::process::exit(1);
    }
}
