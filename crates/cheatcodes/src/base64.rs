use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_sol_types::SolValue;
use base64::prelude::*;
use foundry_evm_core::evm::FoundryEvmNetwork;

fn encode_base64(data: impl AsRef<[u8]>) -> Result {
    Ok(BASE64_STANDARD.encode(data).abi_encode())
}

fn encode_base64_url(data: impl AsRef<[u8]>) -> Result {
    Ok(BASE64_URL_SAFE.encode(data).abi_encode())
}

impl Cheatcode for toBase64_0Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { data } = self;
        encode_base64(data)
    }
}

impl Cheatcode for toBase64_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { data } = self;
        encode_base64(data)
    }
}

impl Cheatcode for toBase64URL_0Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { data } = self;
        encode_base64_url(data)
    }
}

impl Cheatcode for toBase64URL_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { data } = self;
        encode_base64_url(data)
    }
}
