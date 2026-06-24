/// Prints a message to [`stderr`][std::io::stderr] and reads a line from stdin into a String.
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
        let _ = $crate::sh_eprint!($($tt)+);
        match ::std::io::Write::flush(&mut ::std::io::stderr()) {
            ::core::result::Result::Ok(()) => $crate::prompt!(),
            ::core::result::Result::Err(e) => ::core::result::Result::Err(::eyre::eyre!("Could not flush stderr: {e}"))
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

/// Prints a status message to stderr with a trailing newline.
///
/// Use for human-facing diagnostic prose ("Compiling…", "Deploying contract…")
/// that is not the command's primary machine-readable result.
#[macro_export]
macro_rules! sh_status {
    ($($args:tt)*) => {
        $crate::sh_eprintln!($($args)*)
    };
}

/// Prints a progress message to stderr with a trailing newline.
///
/// Use for transient progress updates outside the spinner.
///
/// Suppressed when:
/// - `--quiet` is set, or
/// - stderr is not a tty (e.g. CI logs, piped consumers).
///
/// Always returns `Ok(())`; progress is best-effort and never fails the caller.
#[macro_export]
macro_rules! sh_progress {
    ($($args:tt)*) => {{
        if $crate::shell::is_err_tty() && !$crate::shell::is_quiet() {
            let _ = $crate::sh_eprintln!($($args)*);
        }
        ::core::result::Result::<(), ::eyre::Report>::Ok(())
    }};
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

        sh_status!("status")?;
        sh_status!("status {}", "arg")?;

        sh_progress!("progress")?;
        sh_progress!("progress {}", "arg")?;

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

    /// Asserts that every macro routes to the channel documented in
    /// `docs/dev/output-channels.md`.
    #[test]
    fn routing_contract() -> eyre::Result<()> {
        let mut shell = crate::Shell::captured();

        // stdout: machine-readable result
        sh_print!(&mut shell, "out-print")?;
        sh_println!(&mut shell, "out-println")?;

        // stderr: diagnostics + raw stderr
        sh_eprint!(&mut shell, "err-print")?;
        sh_eprintln!(&mut shell, "err-println")?;
        crate::Shell::warn(&mut shell, "warn-msg")?;
        crate::Shell::error(&mut shell, "err-msg")?;

        let stdout = std::str::from_utf8(shell.captured_stdout().unwrap()).unwrap();
        let stderr = std::str::from_utf8(shell.captured_stderr().unwrap()).unwrap();

        // stdout only contains what `sh_print!`/`sh_println!` produced.
        assert_eq!(stdout, "out-printout-println\n");

        // stderr received the eprint/warn/error output and no stdout content.
        assert!(stderr.contains("err-print"), "stderr missing eprint: {stderr:?}");
        assert!(stderr.contains("err-println"), "stderr missing eprintln: {stderr:?}");
        assert!(stderr.contains("warn-msg"), "stderr missing warn: {stderr:?}");
        assert!(stderr.contains("err-msg"), "stderr missing error: {stderr:?}");
        assert!(!stderr.contains("out-print"), "stdout content leaked to stderr: {stderr:?}");
        assert!(!stderr.contains("out-println"), "stdout content leaked to stderr: {stderr:?}");

        Ok(())
    }

    /// `--quiet` currently suppresses both stdout and stderr diagnostics, but `sh_err!` must
    /// always be visible. The stdout half of this is intentional for now; it will be flipped
    /// to "stdout is never suppressed" once the prose `sh_println!` call sites in forge/script
    /// are migrated to `sh_status!` (see `docs/dev/output-channels.md`).
    #[test]
    fn quiet_contract() -> eyre::Result<()> {
        let mut shell = crate::Shell::captured();
        shell.set_output_mode(crate::shell::OutputMode::Quiet);

        sh_println!(&mut shell, "result")?;
        sh_eprintln!(&mut shell, "diag")?;
        crate::Shell::warn(&mut shell, "warned")?;
        crate::Shell::error(&mut shell, "boom")?;

        let stdout = std::str::from_utf8(shell.captured_stdout().unwrap()).unwrap();
        let stderr = std::str::from_utf8(shell.captured_stderr().unwrap()).unwrap();

        // Today's behavior: stdout is suppressed by --quiet. Pinned here so the future
        // migration that flips this bypass has to deliberately update the test.
        assert!(stdout.is_empty(), "stdout leaked through --quiet: {stdout:?}");
        assert!(!stderr.contains("diag"), "eprintln leaked through --quiet: {stderr:?}");
        assert!(!stderr.contains("warned"), "warn leaked through --quiet: {stderr:?}");
        assert!(stderr.contains("boom"), "sh_err was suppressed by --quiet: {stderr:?}");

        Ok(())
    }
}
