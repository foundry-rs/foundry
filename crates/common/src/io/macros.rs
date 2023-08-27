/// Prints a message to [`stdout`][io::stdout] and [reads a line from stdin into a String](read).
///
/// Returns `Result<T>`, so sometimes `T` must be explicitly specified, like in `str::parse`.
///
/// # Examples
///
/// ```no_run
/// # use foundry_cli::prompt;
/// let response: String = prompt!("Would you like to continue? [y/N] ")?;
/// if !matches!(response.as_str(), "y" | "Y") {
///     return Ok(())
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[macro_export]
macro_rules! prompt {
    () => {
        $crate::stdin::parse_line()
    };

    ($($tt:tt)+) => {{
        ::std::print!($($tt)+);
        match ::std::io::Write::flush(&mut ::std::io::stdout()) {
            ::core::result::Result::Ok(()) => $crate::prompt!(),
            ::core::result::Result::Err(e) => ::core::result::Result::Err(::eyre::eyre!("Could not flush stdout: {e}"))
        }
    }};
}

/// Prints a formatted error to stderr.
#[macro_export]
macro_rules! sh_err {
    ($($args:tt)*) => {
        $crate::Shell::error(&mut *$crate::Shell::get(), ::core::format_args!($($args)*))
    };
}

/// Prints a formatted warning to stderr.
#[macro_export]
macro_rules! sh_warn {
    ($($args:tt)*) => {
        $crate::Shell::warn(&mut *$crate::Shell::get(), ::core::format_args!($($args)*))
    };
}

/// Prints a formatted note to stderr.
#[macro_export]
macro_rules! sh_note {
    ($($args:tt)*) => {
        $crate::Shell::note(&mut *$crate::Shell::get(), ::core::format_args!($($args)*))
    };
}

/// Prints a raw formatted message to stdout.
#[macro_export]
macro_rules! sh_print {
    ($($args:tt)*) => {
        $crate::Shell::print_out(&mut *$crate::Shell::get(), ::core::format_args!($($args)*))
    };
}

/// Prints a raw formatted message to stderr.
#[macro_export]
macro_rules! sh_eprint {
    ($($args:tt)*) => {
        $crate::Shell::print_err(&mut *$crate::Shell::get(), ::core::format_args!($($args)*))
    };
}

/// Prints a raw formatted message to stdout, with a trailing newline.
#[macro_export]
macro_rules! sh_println {
    () => {
        $crate::sh_print!("\n")
    };

    ($($t:tt)*) => {
        $crate::sh_print!("{}\n", ::core::format_args!($($t)*))
    };
}

/// Prints a raw formatted message to stderr, with a trailing newline.
#[macro_export]
macro_rules! sh_eprintln {
    () => {
        $crate::sh_eprint!("\n")
    };

    ($($t:tt)+) => {
        $crate::sh_eprint!("{}\n", ::core::format_args!($($t)+))
    };
}

#[cfg(test)]
mod tests {
    use crate::Shell;

    #[test]
    fn macros() {
        Shell::new().set();

        sh_err!("err").unwrap();
        sh_err!("err {}", "arg").unwrap();

        sh_warn!("warn").unwrap();
        sh_warn!("warn {}", "arg").unwrap();

        sh_note!("note").unwrap();
        sh_note!("note {}", "arg").unwrap();

        sh_print!("print -").unwrap();
        sh_print!("print {} -", "arg").unwrap();

        sh_println!("println").unwrap();
        sh_println!("println {}", "arg").unwrap();

        sh_eprint!("eprint -").unwrap();
        sh_eprint!("eprint {} -", "arg").unwrap();

        sh_eprintln!("eprintln").unwrap();
        sh_eprintln!("eprintln {}", "arg").unwrap();
    }
}
