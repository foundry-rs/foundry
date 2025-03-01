use clap::{builder::PossibleValue, value_parser, Arg, ArgAction, ColorChoice, Command, ValueEnum};

use crate::{
    config::{ParsedSeconds, SortingAttr},
    counter::MaxCountUInt,
    time::TimerKind,
};

pub(crate) fn command() -> Command {
    fn option(name: &'static str) -> Arg {
        Arg::new(name).long(name)
    }

    fn flag(name: &'static str) -> Arg {
        option(name).action(ArgAction::SetTrue)
    }

    fn ignored_flag(name: &'static str) -> Arg {
        flag(name).hide(true)
    }

    // Custom arguments not supported by libtest:
    // - bytes-format
    // - sample-count
    // - sample-size
    // - timer
    // - sort
    // - sortr

    // TODO: `--format <pretty|terse>`

    Command::new("divan")
        .arg(
            Arg::new("filter")
                .value_name("FILTER")
                .help("Only run benchmarks whose names match this pattern")
                .action(ArgAction::Append),
        )
        .arg(
            flag("test")
                .help("Run benchmarks once to ensure they run successfully")
                .conflicts_with("list"),
        )
        .arg(flag("list").help("Lists benchmarks").conflicts_with("test"))
        .arg(
            option("color")
                .value_name("WHEN")
                .help("Controls when to use colors")
                .value_parser(value_parser!(ColorChoice))
        )
        .arg(
            option("skip")
                .value_name("FILTER")
                .help("Skip benchmarks whose names match this pattern")
                .action(ArgAction::Append),
        )
        .arg(flag("exact").help("Filter benchmarks by exact name rather than by pattern"))
        .arg(flag("ignored").help("Run only ignored benchmarks").conflicts_with("include-ignored"))
        .arg(
            flag("include-ignored")
                .help("Run ignored and not-ignored benchmarks")
                .conflicts_with("ignored"),
        )
        .arg(
            option("sort")
                .env("DIVAN_SORT")
                .value_name("ATTRIBUTE")
                .help("Sort benchmarks in ascending order")
                .value_parser(value_parser!(SortingAttr))
        )
        .arg(
            option("sortr")
                .env("DIVAN_SORTR")
                .value_name("ATTRIBUTE")
                .help("Sort benchmarks in descending order")
                .value_parser(value_parser!(SortingAttr))
                .overrides_with("sort"),
        )
        .arg(
            option("timer")
                .env("DIVAN_TIMER")
                .value_name("os|tsc")
                .help("Set the timer used for measuring samples")
                .value_parser(value_parser!(TimerKind)),
        )
        .arg(
            option("sample-count")
                .env("DIVAN_SAMPLE_COUNT")
                .value_name("N")
                .help("Set the number of sampling iterations")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            option("sample-size")
                .env("DIVAN_SAMPLE_SIZE")
                .value_name("N")
                .help("Set the number of iterations inside a single sample")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            option("threads")
                .env("DIVAN_THREADS")
                .value_name("N")
                .value_delimiter(',')
                .action(ArgAction::Append)
                .help("Run across multiple threads to measure contention on atomics and locks")
                .value_parser(value_parser!(usize)),
        )
        .arg(
            option("min-time")
                .env("DIVAN_MIN_TIME")
                .value_name("SECS")
                .help("Set the minimum seconds spent benchmarking a single function")
                .value_parser(value_parser!(ParsedSeconds)),
        )
        .arg(
            option("max-time")
                .env("DIVAN_MAX_TIME")
                .value_name("SECS")
                .help("Set the maximum seconds spent benchmarking a single function, with priority over '--min-time'")
                .value_parser(value_parser!(ParsedSeconds)),
        )
        .arg(
            option("skip-ext-time")
                .env("DIVAN_SKIP_EXT_TIME")
                .value_name("true|false")
                .help("When '--min-time' or '--max-time' is set, skip time external to benchmarked functions")
                .value_parser(value_parser!(bool))
                .num_args(0..=1),
        )
        .arg(
            option("items-count")
                .env("DIVAN_ITEMS_COUNT")
                .value_name("N")
                .help("Set every benchmark to have a throughput of N items")
                .value_parser(value_parser!(MaxCountUInt)),
        )
        .arg(
            option("bytes-count")
                .env("DIVAN_BYTES_COUNT")
                .value_name("N")
                .help("Set every benchmark to have a throughput of N bytes")
                .value_parser(value_parser!(MaxCountUInt)),
        )
        .arg(
            option("bytes-format")
                .env("DIVAN_BYTES_FORMAT")
                .help("Set the numerical base for bytes in output")
                .value_name("decimal|binary")
                .value_parser(value_parser!(crate::counter::PrivBytesFormat))
        )
        .arg(
            option("chars-count")
                .env("DIVAN_CHARS_COUNT")
                .value_name("N")
                .help("Set every benchmark to have a throughput of N string scalars")
                .value_parser(value_parser!(MaxCountUInt)),
        )
        .arg(
            option("cycles-count")
                .env("DIVAN_CYCLES_COUNT")
                .value_name("N")
                .help("Set every benchmark to have a throughput of N cycles, displayed as Hertz")
                .value_parser(value_parser!(MaxCountUInt)),
        )
        // ignored:
        .args([ignored_flag("bench"), ignored_flag("nocapture"), ignored_flag("show-output")])
}

impl ValueEnum for TimerKind {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Os, Self::Tsc]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        let name = match self {
            Self::Os => "os",
            Self::Tsc => "tsc",
        };
        Some(PossibleValue::new(name))
    }
}

impl ValueEnum for SortingAttr {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Kind, Self::Name, Self::Location]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        let name = match self {
            Self::Kind => "kind",
            Self::Name => "name",
            Self::Location => "location",
        };
        Some(PossibleValue::new(name))
    }
}
