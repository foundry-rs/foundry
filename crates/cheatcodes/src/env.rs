//! Implementations of [`Environment`](spec::Group::Environment) cheatcodes.

use crate::{Cheatcode, Error, Result, Vm::*, string};
use alloy_dyn_abi::DynSolType;
use alloy_sol_types::SolValue;
use std::{env, sync::OnceLock};

/// Stores the forge execution context for the duration of the program.
pub static FORGE_CONTEXT: OnceLock<ForgeContext> = OnceLock::new();

/// Returns the current forge execution context, if it has been set.
pub fn current_execution_context() -> Option<ForgeContext> {
    FORGE_CONTEXT.get().copied()
}

impl Cheatcode for setEnvCall {
    fn apply_context_free(&self) -> Result {
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
            unsafe {
                env::set_var(key, value);
            }
            Ok(Default::default())
        }
    }
}

impl Cheatcode for resolveEnvCall {
    fn apply_context_free(&self) -> Result {
        let Self { input } = self;
        let resolved = foundry_config::resolve::interpolate(input)
            .map_err(|e| fmt_err!("failed to resolve env var: {e}"))?;
        Ok(resolved.abi_encode())
    }
}

impl Cheatcode for envExistsCall {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        Ok(env::var(name).is_ok().abi_encode())
    }
}

impl Cheatcode for envBool_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Bool)
    }
}

impl Cheatcode for envUint_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Uint(256))
    }
}

impl Cheatcode for envInt_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Int(256))
    }
}

impl Cheatcode for envAddress_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Address)
    }
}

impl Cheatcode for envBytes32_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for envString_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::String)
    }
}

impl Cheatcode for envBytes_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name } = self;
        env(name, &DynSolType::Bytes)
    }
}

impl Cheatcode for envBool_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Bool)
    }
}

impl Cheatcode for envUint_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Uint(256))
    }
}

impl Cheatcode for envInt_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Int(256))
    }
}

impl Cheatcode for envAddress_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Address)
    }
}

impl Cheatcode for envBytes32_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for envString_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::String)
    }
}

impl Cheatcode for envBytes_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim } = self;
        env_array(name, delim, &DynSolType::Bytes)
    }
}

// bool
impl Cheatcode for envOr_0Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Bool)
    }
}

// uint256
impl Cheatcode for envOr_1Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Uint(256))
    }
}

// int256
impl Cheatcode for envOr_2Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Int(256))
    }
}

// address
impl Cheatcode for envOr_3Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Address)
    }
}

// bytes32
impl Cheatcode for envOr_4Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::FixedBytes(32))
    }
}

// string
impl Cheatcode for envOr_5Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::String)
    }
}

// bytes
impl Cheatcode for envOr_6Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, defaultValue } = self;
        env_default(name, defaultValue, &DynSolType::Bytes)
    }
}

// bool[]
impl Cheatcode for envOr_7Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Bool)
    }
}

// uint256[]
impl Cheatcode for envOr_8Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Uint(256))
    }
}

// int256[]
impl Cheatcode for envOr_9Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Int(256))
    }
}

// address[]
impl Cheatcode for envOr_10Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::Address)
    }
}

// bytes32[]
impl Cheatcode for envOr_11Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::FixedBytes(32))
    }
}

// string[]
impl Cheatcode for envOr_12Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim, defaultValue } = self;
        env_array_default(name, delim, defaultValue, &DynSolType::String)
    }
}

// bytes[]
impl Cheatcode for envOr_13Call {
    fn apply_context_free(&self) -> Result {
        let Self { name, delim, defaultValue } = self;
        let default = defaultValue.clone();
        env_array_default(name, delim, &default, &DynSolType::Bytes)
    }
}

impl Cheatcode for isContextCall {
    fn apply_context_free(&self) -> Result {
        let Self { context } = self;
        Ok((FORGE_CONTEXT.get() == Some(context)).abi_encode())
    }
}

/// Set `forge` command current execution context for the duration of the program.
/// Execution context is immutable, subsequent calls of this function won't change the context.
pub fn set_execution_context(context: ForgeContext) {
    let _ = FORGE_CONTEXT.set(context);
}

fn env(key: &str, ty: &DynSolType) -> Result {
    get_env(key).and_then(|val| string::parse(&val, ty).map_err(|_| parse_error(key, ty)))
}

fn env_default<T: SolValue>(key: &str, default: &T, ty: &DynSolType) -> Result {
    if env_is_missing(key) {
        return Ok(default.abi_encode());
    }
    let val = get_env(key)?;
    // An empty string is a valid `string` value, but it can never parse as any other type, so a
    // variable that is set but empty is treated as not found for the other typed overloads.
    if val.is_empty() && !matches!(ty, DynSolType::String) {
        return Ok(default.abi_encode());
    }
    string::parse(&val, ty).map_err(|_| parse_error(key, ty))
}

fn env_array(key: &str, delim: &str, ty: &DynSolType) -> Result {
    get_env(key).and_then(|val| {
        let values = || val.split(delim).map(str::trim);
        string::parse_array(values(), ty).map_err(|_| {
            // Only the position of the first element that fails to parse is reported.
            let index = values().position(|value| string::parse_value(value, ty).is_err());
            fmt_err!("failed parsing ${key}[{}] as type `{ty}`", index.unwrap_or_default())
        })
    })
}

fn env_array_default<T: SolValue>(key: &str, delim: &str, default: &T, ty: &DynSolType) -> Result {
    if env_is_missing(key) {
        return Ok(default.abi_encode());
    }
    env_array(key, delim, ty)
}

/// Returns `true` if the variable is not set at all. The `envOr` cheatcodes fall back to their
/// default only then (or when the variable is set but empty, for the scalar typed overloads); a
/// variable that is set to a value that cannot be parsed is an error.
fn env_is_missing(key: &str) -> bool {
    matches!(env::var(key), Err(env::VarError::NotPresent))
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

/// Returns the error for a value of `key` that does not parse as `ty`.
///
/// The parser's own message is dropped because it quotes the value, which may be a secret.
fn parse_error(key: &str, ty: &DynSolType) -> Error {
    fmt_err!("failed parsing ${key} as type `{ty}`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_or_returns_default_when_missing() {
        let key = "env_or_missing";
        unsafe {
            env::remove_var(key);
        }
        let default = 7u64;
        let encoded = default.abi_encode();
        assert_eq!(env_default(key, &default, &DynSolType::Uint(256)).unwrap(), encoded);
        assert_eq!(env_array_default(key, ",", &default, &DynSolType::Uint(256)).unwrap(), encoded);
    }

    #[test]
    fn env_or_returns_default_when_empty() {
        let key = "env_or_empty";
        unsafe {
            env::set_var(key, "");
        }
        let default = 7u64;
        assert_eq!(
            env_default(key, &default, &DynSolType::Uint(256)).unwrap(),
            default.abi_encode()
        );
        assert_eq!(
            env_array_default(key, ",", &default, &DynSolType::Uint(256)).unwrap(),
            Vec::<u64>::new().abi_encode()
        );
        // An empty string is a valid `string` value and must not be replaced by the default.
        assert_eq!(
            env_default(key, &"default", &DynSolType::String).unwrap(),
            String::new().abi_encode()
        );
        unsafe {
            env::remove_var(key);
        }
    }

    #[test]
    fn env_or_rejects_unparsable_value() {
        let key = "env_or_unparsable";
        unsafe {
            env::set_var(key, "not_a_number");
        }

        let err = env_default(key, &7u64, &DynSolType::Uint(256)).unwrap_err().to_string();
        assert!(err.contains("$env_or_unparsable"), "{err:?}");
        let err =
            env_array_default(key, ",", &7u64, &DynSolType::Uint(256)).unwrap_err().to_string();
        assert!(err.contains("$env_or_unparsable"), "{err:?}");
        unsafe {
            env::remove_var(key);
        }
    }

    #[test]
    fn parse_env_uint() {
        let key = "parse_env_uint";
        let value = "t";
        unsafe {
            env::set_var(key, value);
        }

        let err = env(key, &DynSolType::Uint(256)).unwrap_err().to_string();
        assert_eq!(err, "failed parsing $parse_env_uint as type `uint256`");
        unsafe {
            env::remove_var(key);
        }
    }

    #[test]
    fn parse_env_errors_do_not_leak_values() {
        let key = "parse_env_errors_do_not_leak_values";
        let uint = DynSolType::Uint(256);
        for (value, index) in [
            ("1, secretvalue", 1),
            ("1,\"1\"secret", 1),
            ("ff,0x", 0),
            ("secret\\value", 0),
            ("secret\nvalue", 0),
            ("1,secret\nvalue", 1),
        ] {
            unsafe {
                env::set_var(key, value);
            }
            let scalar = env(key, &uint).unwrap_err().to_string();
            assert_eq!(scalar, format!("failed parsing ${key} as type `uint256`"));
            let array = env_array(key, ",", &uint).unwrap_err().to_string();
            assert_eq!(array, format!("failed parsing ${key}[{index}] as type `uint256`"));
            let array_default = env_array_default(key, ",", &7u64, &uint).unwrap_err().to_string();
            assert_eq!(array_default, array);
        }
        unsafe {
            env::remove_var(key);
        }
    }
}
