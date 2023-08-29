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
        $crate::__sh_dispatch!(error $($args)*)
    };
}

/// Prints a formatted warning to stderr.
#[macro_export]
macro_rules! sh_warn {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(warn $($args)*)
    };
}

/// Prints a formatted note to stderr.
#[macro_export]
macro_rules! sh_note {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(note $($args)*)
    };
}

/// Prints a raw formatted message to stdout.
///
/// **Note**: This macro is **not** affected by the `--quiet` flag.
#[macro_export]
macro_rules! sh_print {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(print_out $($args)*)
    };
}

/// Prints a raw formatted message to stderr.
///
/// **Note**: This macro **is** affected by the `--quiet` flag.
#[macro_export]
macro_rules! sh_eprint {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(print_err $($args)*)
    };
}

/// Prints a raw formatted message to stdout, with a trailing newline.
///
/// **Note**: This macro is **not** affected by the `--quiet` flag.
#[macro_export]
macro_rules! sh_println {
    () => {
        $crate::sh_print!("\n")
    };

    ($fmt:literal $($args:tt)*) => {
        $crate::sh_print!("{}\n", ::core::format_args!($fmt $($args)*))
    };

    ($shell:expr $(,)?) => {
        $crate::sh_print!($shell, "\n")
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
/// **Note**: This macro **is** affected by the `--quiet` flag.
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

/// Prints a justified status header with an optional message.
#[macro_export]
macro_rules! sh_status {
    ($header:expr) => {
        $crate::Shell::status_header(&mut *$crate::Shell::get(), $header)
    };

    ($header:expr => $($args:tt)*) => {
        $crate::Shell::status(&mut *$crate::Shell::get(), $header, ::core::format_args!($($args)*))
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __sh_dispatch {
    ($f:ident $fmt:literal $($args:tt)*) => {
        $crate::Shell::$f(&mut *$crate::Shell::get(), ::core::format_args!($fmt $($args)*))
    };

    ($f:ident $shell:expr, $($args:tt)*) => {
        $crate::Shell::$f($shell, ::core::format_args!($($args)*))
    };

    ($f:ident $($args:tt)*) => {
        $crate::Shell::$f(&mut *$crate::Shell::get(), ::core::format_args!($($args)*))
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn macros() {
        sh_err!("err").unwrap();
        sh_err!("err {}", "arg").unwrap();

        sh_warn!("warn").unwrap();
        sh_warn!("warn {}", "arg").unwrap();

        sh_note!("note").unwrap();
        sh_note!("note {}", "arg").unwrap();

        sh_print!("print -").unwrap();
        sh_print!("print {} -", "arg").unwrap();

        sh_println!().unwrap();
        sh_println!("println").unwrap();
        sh_println!("println {}", "arg").unwrap();

        sh_eprint!("eprint -").unwrap();
        sh_eprint!("eprint {} -", "arg").unwrap();

        sh_eprintln!().unwrap();
        sh_eprintln!("eprintln").unwrap();
        sh_eprintln!("eprintln {}", "arg").unwrap();
    }

    #[test]
    fn macros_with_shell() {
        let shell = &mut crate::Shell::new();
        sh_eprintln!(shell).unwrap();
        sh_eprintln!(shell,).unwrap();
        sh_eprintln!(shell, "shelled eprintln").unwrap();
        sh_eprintln!(shell, "shelled eprintln {}", "arg").unwrap();
        sh_eprintln!(&mut crate::Shell::new(), "shelled eprintln {}", "arg").unwrap();
    }
}
