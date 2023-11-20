//! Implementations of [`Environment`](crate::Group::Environment) cheatcodes.

use crate::{string, Cheatcode, Cheatcodes, Error, Result, Vm::*};
use alloy_dyn_abi::DynSolType;
use alloy_primitives::Bytes;
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
        env(name, &DynSolType::Bool)
    }
}

impl Cheatcode for envUint_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Uint(256))
    }
}

impl Cheatcode for envInt_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Int(256))
    }
}

impl Cheatcode for envAddress_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Address)
    }
}

impl Cheatcode for envBytes32_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for envString_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::String)
    }
}

impl Cheatcode for envBytes_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Bytes)
    }
}

impl Cheatcode for envBool_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Bool)
    }
}

impl Cheatcode for envUint_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Uint(256))
    }
}

impl Cheatcode for envInt_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Int(256))
    }
}

impl Cheatcode for envAddress_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Address)
    }
}

impl Cheatcode for envBytes32_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for envString_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::String)
    }
}

impl Cheatcode for envBytes_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Bytes)
    }
}

// bool
impl Cheatcode for envOr_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Bool)
    }
}

// uint256
impl Cheatcode for envOr_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Uint(256))
    }
}

// int256
impl Cheatcode for envOr_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Int(256))
    }
}

// address
impl Cheatcode for envOr_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Address)
    }
}

// bytes32
impl Cheatcode for envOr_4Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::FixedBytes(32))
    }
}

// string
impl Cheatcode for envOr_5Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::String)
    }
}

// bytes
impl Cheatcode for envOr_6Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Bytes)
    }
}

// bool[]
impl Cheatcode for envOr_7Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Bool)
    }
}

// uint256[]
impl Cheatcode for envOr_8Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Uint(256))
    }
}

// int256[]
impl Cheatcode for envOr_9Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Int(256))
    }
}

// address[]
impl Cheatcode for envOr_10Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Address)
    }
}

// bytes32[]
impl Cheatcode for envOr_11Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::FixedBytes(32))
    }
}

// string[]
impl Cheatcode for envOr_12Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::String)
    }
}

// bytes[]
impl Cheatcode for envOr_13Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name, delim, defaultValue } = self;
        let default = defaultValue.iter().map(|vec| vec.clone().into()).collect::<Vec<Bytes>>();
        env_array_default(name, delim, &default, &DynSolType::Bytes)
    }
}

fn env(key: &str, ty: &DynSolType) -> Result {
    get_env(key).and_then(|val| string::parse(&val, ty).map_err(map_env_err(key, &val)))
}

fn env_default<T: SolValue>(key: &str, default: &T, ty: &DynSolType) -> Result {
    Ok(env(key, ty).unwrap_or_else(|_| default.abi_encode()))
}

fn env_array(key: &str, delim: &str, ty: &DynSolType) -> Result {
    get_env(key).and_then(|val| {
        string::parse_array(val.split(delim).map(str::trim), ty).map_err(map_env_err(key, &val))
    })
}

fn env_array_default<T: SolValue>(key: &str, delim: &str, default: &T, ty: &DynSolType) -> Result {
    Ok(env_array(key, delim, ty).unwrap_or_else(|_| default.abi_encode()))
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

/// Converts the error message of a failed parsing attempt to a more user-friendly message that
/// doesn't leak the value.
fn map_env_err<'a>(key: &'a str, value: &'a str) -> impl FnOnce(Error) -> Error + 'a {
    move |e| {
        let e = e.to_string(); // failed parsing \"xy(123)\" as type `uint256`: parser error:\nxy(123)\n ^\nexpected at
                               // least one digit
        let mut e = e.as_str();
        // cut off the message to not leak the value
        let sep = if let Some(idx) = e.rfind(" as type `") {
            e = &e[idx..];
            ""
        } else {
            ": "
        };
        // ensure we're also removing the value from the underlying alloy parser error message, See
        // [alloy_dyn_abi::parser::Error::parser]
        let e = e.replacen(&format!("\n{value}\n"), &format!("\n${key}\n"), 1);
        fmt_err!("failed parsing ${key}{sep}{e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_uint() {
        let key = "parse_env_uint";
        let value = "xy(123)";
        env::set_var(key, value);

        let err = env(key, &DynSolType::Uint(256)).unwrap_err().to_string();
        assert!(!err.contains(value));
        assert_eq!(err, "failed parsing $parse_env_uint as type `uint256`: parser error:$parse_env_uint ^\nexpected at least one digit");
        env::remove_var(key);
    }
}
