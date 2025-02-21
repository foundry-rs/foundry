use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use base64::prelude::*;

impl Cheatcode for toBase64_0Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { data } = self;
        Ok(BASE64_STANDARD.encode(data))
    }
}

impl Cheatcode for toBase64_1Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { data } = self;
        Ok(BASE64_STANDARD.encode(data))
    }
}

impl Cheatcode for toBase64URL_0Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { data } = self;
        Ok(BASE64_URL_SAFE.encode(data))
    }
}

impl Cheatcode for toBase64URL_1Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { data } = self;
        Ok(BASE64_URL_SAFE.encode(data))
    }
}
