//! Implementations of [`Environment`](crate::Group::Environment) cheatcodes.

use super::{string, Cheatcode, Result};
use crate::{Cheatcodes, Vm::*};
use alloy_primitives::{Address, Bytes, B256, I256, U256};
use alloy_sol_types::SolValue;
use std::env;

impl Cheatcode for setEnvCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name: key, value } = self;
        if key.is_empty() {
            Err(fmt_err!("environment variable key can't be empty"))
        } else if key.contains('=') {
            Err(fmt_err!("environment variable key can't contain equal sign `=`"))
        } else if key.contains('\0') {
            Err(fmt_err!("environment variable key can't contain NUL character `\\0`"))
        } else if value.contains('\0') {
            Err(fmt_err!("environment variable value can't contain NUL character `\\0`"))
        } else {
            env::set_var(key, value);
            Ok(Default::default())
        }
    }
}

impl Cheatcode for envBool_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env::<bool>(name, None)
    }
}

impl Cheatcode for envUint_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env::<U256>(name, None)
    }
}

impl Cheatcode for envInt_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env::<I256>(name, None)
    }
}

impl Cheatcode for envAddress_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env::<Address>(name, None)
    }
}

impl Cheatcode for envBytes32_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env::<B256>(name, None)
    }
}

impl Cheatcode for envString_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env::<String>(name, None)
    }
}

impl Cheatcode for envBytes_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env::<Bytes>(name, None)
    }
}

impl Cheatcode for envBool_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array::<bool>(name, delim, None)
    }
}

impl Cheatcode for envUint_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array::<U256>(name, delim, None)
    }
}

impl Cheatcode for envInt_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array::<I256>(name, delim, None)
    }
}

impl Cheatcode for envAddress_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array::<Address>(name, delim, None)
    }
}

impl Cheatcode for envBytes32_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array::<B256>(name, delim, None)
    }
}

impl Cheatcode for envString_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array::<String>(name, delim, None)
    }
}

impl Cheatcode for envBytes_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array::<Bytes>(name, delim, None)
    }
}

// bool
impl Cheatcode for envOr_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env::<bool>(name, Some(defaultValue))
    }
}

// uint256
impl Cheatcode for envOr_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env::<U256>(name, Some(defaultValue))
    }
}

// int256
impl Cheatcode for envOr_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env::<I256>(name, Some(defaultValue))
    }
}

// address
impl Cheatcode for envOr_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env::<Address>(name, Some(defaultValue))
    }
}

// bytes32
impl Cheatcode for envOr_4Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env::<B256>(name, Some(defaultValue))
    }
}

// string
impl Cheatcode for envOr_5Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env::<String>(name, Some(defaultValue))
    }
}

// bytes
impl Cheatcode for envOr_6Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env::<Bytes>(name, Some(&defaultValue.clone().into()))
    }
}

// bool[]
impl Cheatcode for envOr_7Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array::<bool>(name, delim, Some(defaultValue))
    }
}

// uint256[]
impl Cheatcode for envOr_8Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array::<U256>(name, delim, Some(defaultValue))
    }
}

// int256[]
impl Cheatcode for envOr_9Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array::<I256>(name, delim, Some(defaultValue))
    }
}

// address[]
impl Cheatcode for envOr_10Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array::<Address>(name, delim, Some(defaultValue))
    }
}

// bytes32[]
impl Cheatcode for envOr_11Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array::<B256>(name, delim, Some(defaultValue))
    }
}

// string[]
impl Cheatcode for envOr_12Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array::<String>(name, delim, Some(defaultValue))
    }
}

// bytes[]
impl Cheatcode for envOr_13Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        let default = defaultValue.iter().map(|vec| vec.clone().into()).collect::<Vec<Bytes>>();
        env_array::<Bytes>(name, delim, Some(&default))
    }
}

fn env<T>(key: &str, default: Option<&T>) -> Result
where
    T: SolValue + std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match (get_env(key), default) {
        (Ok(val), _) => string::parse::<T>(&val),
        (Err(_), Some(default)) => Ok(default.abi_encode()),
        (Err(e), None) => Err(e),
    }
}

fn env_array<T>(key: &str, delim: &str, default: Option<&[T]>) -> Result
where
    T: SolValue + std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match (get_env(key), default) {
        (Ok(val), _) => string::parse_array::<_, _, T>(val.split(delim).map(str::trim)),
        (Err(_), Some(default)) => Ok(default.abi_encode()),
        (Err(e), None) => Err(e),
    }
}

fn get_env(key: &str) -> Result<String> {
    match env::var(key) {
        Ok(val) => Ok(val),
        Err(env::VarError::NotPresent) => Err(fmt_err!("environment variable {key:?} not found")),
        Err(env::VarError::NotUnicode(s)) => {
            Err(fmt_err!("environment variable {key:?} was not valid unicode: {s:?}"))
        }
    }
}
