/// Prints a message to [`stdout`][std::io::stdout] and reads a line from stdin into a String.
///
/// Returns `Result<T>`, so sometimes `T` must be explicitly specified, like in `str::parse`.
///
/// # Examples
///
/// ```no_run
/// use foundry_common::prompt;
///
/// let response: String = prompt!("Would you like to continue? [y/N] ")?;
/// if !matches!(response.as_str(), "y" | "Y") {
///     return Ok(());
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[macro_export]
macro_rules! prompt {
    () => {
        $crate::stdin::parse_line()
    };

    ($($tt:tt)+) => {{
        let _ = $crate::sh_print!($($tt)+);
        match ::std::io::Write::flush(&mut ::std::io::stdout()) {
            ::core::result::Result::Ok(()) => $crate::prompt!(),
            ::core::result::Result::Err(e) => ::core::result::Result::Err(::eyre::eyre!("Could not flush stdout: {e}"))
        }
    }};
}

/// Prints a formatted error to stderr.
///
/// **Note**: will log regardless of the verbosity level.
#[macro_export]
macro_rules! sh_err {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(error $($args)*)
    };
}

/// Prints a formatted warning to stderr.
///
/// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
#[macro_export]
macro_rules! sh_warn {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(warn $($args)*)
    };
}

/// Prints a raw formatted message to stdout.
///
/// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
#[macro_export]
macro_rules! sh_print {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(print_out $($args)*)
    };

    ($shell:expr, $($args:tt)*) => {
        $crate::__sh_dispatch!(print_out $shell, $($args)*)
    };
}

/// Prints a raw formatted message to stderr.
///
/// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
#[macro_export]
macro_rules! sh_eprint {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(print_err $($args)*)
    };

    ($shell:expr, $($args:tt)*) => {
        $crate::__sh_dispatch!(print_err $shell, $($args)*)
    };
}

/// Prints a raw formatted message to stdout, with a trailing newline.
///
/// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
#[macro_export]
macro_rules! sh_println {
    () => {
        $crate::sh_print!("\n")
    };

    ($fmt:literal $($args:tt)*) => {
        $crate::sh_print!("{}\n", ::core::format_args!($fmt $($args)*))
    };

    ($shell:expr $(,)?) => {
        $crate::sh_print!($shell, "\n").expect("failed to write newline")
    };

    ($shell:expr, $($args:tt)*) => {
        $crate::sh_print!($shell, "{}\n", ::core::format_args!($($args)*))
    };

    ($($args:tt)*) => {
        $crate::sh_print!("{}\n", ::core::format_args!($($args)*))
    };
}

/// Prints a raw formatted message to stderr, with a trailing newline.
///
/// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
#[macro_export]
macro_rules! sh_eprintln {
    () => {
        $crate::sh_eprint!("\n")
    };

    ($fmt:literal $($args:tt)*) => {
        $crate::sh_eprint!("{}\n", ::core::format_args!($fmt $($args)*))
    };

    ($shell:expr $(,)?) => {
        $crate::sh_eprint!($shell, "\n")
    };

    ($shell:expr, $($args:tt)*) => {
        $crate::sh_eprint!($shell, "{}\n", ::core::format_args!($($args)*))
    };

    ($($args:tt)*) => {
        $crate::sh_eprint!("{}\n", ::core::format_args!($($args)*))
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __sh_dispatch {
    ($f:ident $fmt:literal $($args:tt)*) => {
        $crate::__sh_dispatch!(@impl $f &mut *$crate::Shell::get(), $fmt $($args)*)
    };

    ($f:ident $shell:expr, $($args:tt)*) => {
        $crate::__sh_dispatch!(@impl $f $shell, $($args)*)
    };

    ($f:ident $($args:tt)*) => {
        $crate::__sh_dispatch!(@impl $f &mut *$crate::Shell::get(), $($args)*)
    };

    // Ensure that the global shell lock is held for as little time as possible.
    // Also avoids deadlocks in case of nested calls.
    (@impl $f:ident $shell:expr, $($args:tt)*) => {
        match format!($($args)*) {
            fmt => $crate::Shell::$f($shell, fmt),
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn macros() -> eyre::Result<()> {
        sh_err!("err")?;
        sh_err!("err {}", "arg")?;

        sh_warn!("warn")?;
        sh_warn!("warn {}", "arg")?;

        sh_print!("print -")?;
        sh_print!("print {} -", "arg")?;

        sh_println!()?;
        sh_println!("println")?;
        sh_println!("println {}", "arg")?;

        sh_eprint!("eprint -")?;
        sh_eprint!("eprint {} -", "arg")?;

        sh_eprintln!()?;
        sh_eprintln!("eprintln")?;
        sh_eprintln!("eprintln {}", "arg")?;

        sh_println!("{:?}", {
            sh_println!("hi")?;
            solar::data_structures::fmt::from_fn(|f| {
                let _ = sh_println!("even more nested");
                write!(f, "hi 2")
            })
        })?;

        Ok(())
    }

    #[test]
    fn macros_with_shell() -> eyre::Result<()> {
        let shell = &mut crate::Shell::new();
        sh_eprintln!(shell)?;
        sh_eprintln!(shell,)?;
        sh_eprintln!(shell, "shelled eprintln")?;
        sh_eprintln!(shell, "shelled eprintln {}", "arg")?;
        sh_eprintln!(&mut crate::Shell::new(), "shelled eprintln {}", "arg")?;

        Ok(())
    }
}
