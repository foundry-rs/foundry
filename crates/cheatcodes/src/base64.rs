use crate::{string, Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::DynSolType;
use base64::prelude::*;

impl Cheatcode for toBase64Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { data } = self;
        string::parse(&BASE64_STANDARD.encode(data), &DynSolType::String)
    }
}

impl Cheatcode for toBase64URLCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { data } = self;
        string::parse(&BASE64_URL_SAFE.encode(data), &DynSolType::String)
    }
}
