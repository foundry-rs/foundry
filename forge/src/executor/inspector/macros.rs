/// Returns from the function on an error, discarding the error.
///
/// Useful for inspectors that read state that might be invalid, but do not want to emit
/// appropriate errors themselves.
macro_rules! try_or_return {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return,
        }
    };
}

/// Returns [Return::Continue] on an error, discarding the error.
///
/// Useful for inspectors that read state that might be invalid, but do not want to emit
/// appropriate errors themselves, instead opting to continue.
macro_rules! try_or_continue {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return Return::Continue,
        }
    };
}

/// Tries to convert a U256 to a usize and returns from the function on an error.
///
/// This is useful for opcodes that deal with the stack where parameters might be invalid and you
/// want to defer error handling to the VM itself.
macro_rules! as_usize_or_return {
    ($v:expr) => {
        if $v.0[1] != 0 || $v.0[2] != 0 || $v.0[3] != 0 {
            return
        } else {
            $v.0[0] as usize
        }
    };
}
