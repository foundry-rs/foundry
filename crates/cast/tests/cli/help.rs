//! CLI tests for help commands.

use super::*;

casttest!(print_short_version, |_prj, cmd| {
    cmd.arg("-V").assert_success().stdout_eq(str![[r#"
cast [..]-[..] ([..] [..])

"#]]);
});

casttest!(print_long_version, |_prj, cmd| {
    cmd.arg("--version").assert_success().stdout_eq(str![[r#"
cast Version: [..]
Commit SHA: [..]
Build Timestamp: [..]
Build Profile: [..]

"#]]);
});

// tests that a non-UTF-8 command-line argument produces a clean error instead of an unrecovered
// panic in `GlobalArgs::check_markdown_help` (which used to call `std::env::args()`, documented to
// panic on invalid Unicode, as the very first statement of every binary's entry point)
#[cfg(unix)]
casttest!(non_utf8_argument_does_not_panic, |prj, _cmd| {
    use std::os::unix::ffi::OsStrExt;

    let bad_arg = std::ffi::OsStr::from_bytes(&[0xff]);
    let output = prj.cast_bin().arg(bad_arg).output().unwrap();

    assert_ne!(
        output.status.code(),
        Some(101),
        "a non-UTF-8 argument must not cause an unrecovered panic (exit code 101); got status {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "a non-UTF-8 argument must not panic; stderr: {stderr}"
    );
});

// tests `--help` is printed to std out
casttest!(print_help, |_prj, cmd| {
    cmd.arg("--help").assert_success().stdout_eq(str![[r#"
A Swiss Army knife for interacting with Ethereum applications from the command line

Usage: cast[..] <COMMAND>

Commands:
...

Options:
  -h, --help
          Print help (see a summary with '-h')

  -j, --threads <THREADS>
          Number of threads to use. Specifying 0 defaults to the number of logical cores
...
          [alias: --jobs]

      --profile <PROFILE>
          The configuration profile to use

  -V, --version
          Print version

Display options:
      --color <COLOR>
          The color of the log messages

          Possible values:
          - auto:   Intelligently guess whether to use color output (default)
          - always: Force color output
          - never:  Force disable color output

      --json
          Format log messages as JSON

      --md
          Format log messages as Markdown

  -q, --quiet
          Do not print log messages

  -v, --verbosity...
          Verbosity level of the log messages.
...
          Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
...
          Depending on the context the verbosity levels have different meanings.
...
          For example, the verbosity levels of the EVM are:
          - 2 (-vv): Print logs for all tests.
          - 3 (-vvv): Print execution traces for failing tests.
          - 4 (-vvvv): Print execution traces for all tests, and setup traces for failing tests.
          - 5 (-vvvvv): Print execution and setup traces for all tests, including storage changes
          and
            backtraces with line numbers.

Find more information in the book: https://getfoundry.sh/cast/overview

"#]]);
});
