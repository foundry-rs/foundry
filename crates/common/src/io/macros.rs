/// Prints a message to [`stdout`][io::stdout] and [reads a line from stdin into a String](read).
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
        let _ = $crate::sh_print!($($tt)+);
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
        $crate::__sh_dispatch!(error $($args)*).expect("failed to write error")
    };
}

/// Prints a formatted warning to stderr.
#[macro_export]
macro_rules! sh_warn {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(warn $($args)*).expect("failed to write warning")
    };
}

/// Prints a formatted note to stderr.
#[macro_export]
macro_rules! sh_note {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(note $($args)*).expect("failed to write note")
    };
}

/// Prints a raw formatted message to stdout.
#[macro_export]
macro_rules! sh_print {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(print_out $($args)*).expect("failed to write output")
    };

    ($shell:expr, $($args:tt)*) => {
        $crate::__sh_dispatch!(print_out $shell, $($args)*).expect("failed to write output")
    };
}

/// Prints a raw formatted message to stderr.
#[macro_export]
macro_rules! sh_eprint {
    ($($args:tt)*) => {
        $crate::__sh_dispatch!(print_err $($args)*).expect("failed to write error")
    };

    ($shell:expr, $($args:tt)*) => {
        $crate::__sh_dispatch!(print_err $shell, $($args)*).expect("failed to write error")
    };
}

/// Prints a raw formatted message to stdout, with a trailing newline.
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
        $crate::sh_print!($shell, "{}\n", ::core::format_args!($($args)*)).expect("failed to write line")
    };

    ($($args:tt)*) => {
        $crate::sh_print!("{}\n", ::core::format_args!($($args)*)).expect("failed to write line")
    };
}

/// Prints a raw formatted message to stderr, with a trailing newline.
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
        $crate::sh_eprint!("{}\n", ::core::format_args!($($args)*)).expect("failed to write line")
    };
}

/// Prints a justified status header with an optional message.
#[macro_export]
macro_rules! sh_status {
    ($header:expr) => {
        $crate::Shell::status_header(&mut *$crate::Shell::get(), $header).expect("failed to write status header")
    };

    ($header:expr => $($args:tt)*) => {
        $crate::Shell::status(&mut *$crate::Shell::get(), $header, ::core::format_args!($($args)*)).expect("failed to write status")
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
        sh_err!("err");
        sh_err!("err {}", "arg");

        sh_warn!("warn");
        sh_warn!("warn {}", "arg");

        sh_print!("print -");
        sh_print!("print {} -", "arg");

        sh_println!();
        sh_println!("println");
        sh_println!("println {}", "arg");

        sh_eprint!("eprint -");
        sh_eprint!("eprint {} -", "arg");

        sh_eprintln!();
        sh_eprintln!("eprintln");
        sh_eprintln!("eprintln {}", "arg");

        sh_status!("status");
        sh_status!("status" => "status {}", "arg");
    }

    #[test]
    fn macros_with_shell() {
        let shell = &mut crate::Shell::new();
        sh_eprintln!(shell);
        sh_eprintln!(shell,);
        sh_eprintln!(shell, "shelled eprintln");
        sh_eprintln!(shell, "shelled eprintln {}", "arg");
        sh_eprintln!(&mut crate::Shell::new(), "shelled eprintln {}", "arg");
    }
}
