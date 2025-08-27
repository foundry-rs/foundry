fn main() {
    #[cfg(feature = "std")]
    {
        polkadot_sdk::substrate_wasm_builder::WasmBuilder::build_using_defaults();
    }
}
