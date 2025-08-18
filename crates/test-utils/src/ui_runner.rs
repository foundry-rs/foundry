use std::path::Path;
use ui_test::{
    spanned::Spanned,
    status_emitter::{Gha, StatusEmitter},
};

/// Test runner based on `ui_test`. Adapted from `https://github.com/paradigmxyz/solar/blob/main/tools/tester/src/lib.rs`.
pub fn run_tests<'a>(cmd: &str, cmd_path: &'a Path, testdata: &'a Path) -> eyre::Result<()> {
    ui_test::color_eyre::install()?;

    let mut args = ui_test::Args::test()?;

    // Fast path for `--list`, invoked by `cargo-nextest`.
    {
        let mut dummy_config = ui_test::Config::dummy();
        dummy_config.with_args(&args);
        if ui_test::nextest::emulate(&mut vec![dummy_config]) {
            return Ok(());
        }
    }

    // Condense output if not explicitly requested.
    let requested_pretty = || std::env::args().any(|x| x.contains("--format"));
    if matches!(args.format, ui_test::Format::Pretty) && !requested_pretty() {
        args.format = ui_test::Format::Terse;
    }

    let config = config(cmd, cmd_path, &args, testdata);

    let text_emitter: Box<dyn StatusEmitter> = args.format.into();
    let gha_emitter = Gha { name: "Foundry Lint UI".to_string(), group: true };
    let status_emitter = (text_emitter, gha_emitter);

    // run tests on all .sol files
    ui_test::run_tests_generic(
        vec![config],
        move |path, _config| Some(path.extension().is_some_and(|ext| ext == "sol")),
        per_file_config,
        status_emitter,
    )?;

    Ok(())
}

fn config<'a>(
    cmd: &str,
    cmd_path: &'a Path,
    args: &ui_test::Args,
    testdata: &'a Path,
) -> ui_test::Config {
    let root = testdata.parent().unwrap();
    assert!(
        testdata.exists(),
        "testdata directory does not exist: {};\n\
         you may need to initialize submodules: `git submodule update --init --checkout`",
        testdata.display()
    );

    let mut config = ui_test::Config {
        host: Some(get_host().to_string()),
        target: None,
        root_dir: testdata.into(),
        program: ui_test::CommandBuilder {
            program: cmd_path.into(),
            args: {
                let args = vec![cmd, "--json", "--root", testdata.to_str().expect("invalid root")];
                args.into_iter().map(Into::into).collect()
            },
            out_dir_flag: None,
            input_file_flag: None,
            envs: vec![],
            cfg_flag: None,
        },
        output_conflict_handling: ui_test::error_on_output_conflict,
        bless_command: Some(format!("cargo nextest run {} -- --bless", module_path!())),
        out_dir: root.join("target").join("ui"),
        comment_start: "//",
        diagnostic_extractor: ui_test::diagnostics::rustc::rustc_diagnostics_extractor,
        ..ui_test::Config::dummy()
    };

    macro_rules! register_custom_flags {
        ($($ty:ty),* $(,)?) => {
            $(
                config.custom_comments.insert(<$ty>::NAME, <$ty>::parse);
                if let Some(default) = <$ty>::DEFAULT {
                    config.comment_defaults.base().add_custom(<$ty>::NAME, default);
                }
            )*
        };
    }
    register_custom_flags![];

    config.comment_defaults.base().exit_status = None.into();
    config.comment_defaults.base().require_annotations = Spanned::dummy(true).into();
    config.comment_defaults.base().require_annotations_for_level =
        Spanned::dummy(ui_test::diagnostics::Level::Warn).into();

    let filters = [
        (ui_test::Match::PathBackslash, b"/".to_vec()),
        #[cfg(windows)]
        (ui_test::Match::Exact(vec![b'\r']), b"".to_vec()),
        #[cfg(windows)]
        (ui_test::Match::Exact(br"\\?\".to_vec()), b"".to_vec()),
        (root.into(), b"ROOT".to_vec()),
    ];
    config.comment_defaults.base().normalize_stderr.extend(filters.iter().cloned());
    config.comment_defaults.base().normalize_stdout.extend(filters);

    let filters: &[(&str, &str)] = &[
        // Erase line and column info.
        (r"\.(\w+):[0-9]+:[0-9]+(: [0-9]+:[0-9]+)?", ".$1:LL:CC"),
    ];
    for &(pattern, replacement) in filters {
        config.filter(pattern, replacement);
    }

    let stdout_filters: &[(&str, &str)] =
        &[(&env!("CARGO_PKG_VERSION").replace(".", r"\."), "VERSION")];
    for &(pattern, replacement) in stdout_filters {
        config.stdout_filter(pattern, replacement);
    }
    let stderr_filters: &[(&str, &str)] = &[];
    for &(pattern, replacement) in stderr_filters {
        config.stderr_filter(pattern, replacement);
    }

    config.with_args(args);
    config
}

fn per_file_config(config: &mut ui_test::Config, file: &Spanned<Vec<u8>>) {
    let Ok(src) = std::str::from_utf8(&file.content) else {
        return;
    };

    assert_eq!(config.comment_start, "//");
    let has_annotations = src.contains("//~");
    config.comment_defaults.base().require_annotations = Spanned::dummy(has_annotations).into();
    let code = if has_annotations && src.contains("ERROR:") { 1 } else { 0 };
    config.comment_defaults.base().exit_status = Spanned::dummy(code).into();
}

fn get_host() -> &'static str {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let mut config = ui_test::Config::dummy();
        config.program = ui_test::CommandBuilder::rustc();
        config.fill_host_and_target().unwrap();
        config.host.unwrap()
    })
}
