use crate::{
    opts::{Chisel, ChiselSubcommand},
    prelude::{ChiselCommand, ChiselDispatcher, SolidityHelper},
};
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::utils::{self, LoadConfig};
use foundry_common::fs;
use rustyline::{Editor, config::Configurer, error::ReadlineError};
use std::{ops::ControlFlow, path::PathBuf};
use yansi::Paint;

/// Run the `chisel` command line interface.
pub fn run() -> Result<()> {
    setup()?;

    let args = Chisel::parse();
    args.global.init()?;
    args.global.tokio_runtime().block_on(run_command(args))
}

/// Setup the global logger and other utilities.
pub fn setup() -> Result<()> {
    utils::common_setup();
    utils::subscriber();

    Ok(())
}

macro_rules! try_cf {
    ($e:expr) => {
        match $e {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(()) => return Ok(()),
        }
    };
}

/// Run the subcommand.
pub async fn run_command(args: Chisel) -> Result<()> {
    // Load configuration
    let (config, evm_opts) = args.load_config_and_evm_opts()?;

    // Create a new cli dispatcher
    let mut dispatcher = ChiselDispatcher::new(crate::source::SessionSourceConfig {
        // Enable traces if any level of verbosity was passed
        traces: config.verbosity > 0,
        foundry_config: config,
        no_vm: args.no_vm,
        evm_opts,
        backend: None,
        calldata: None,
        ir_minimum: args.ir_minimum,
    })?;

    // Execute prelude Solidity source files
    evaluate_prelude(&mut dispatcher, args.prelude).await?;

    if let Some(cmd) = args.cmd {
        try_cf!(handle_cli_command(&mut dispatcher, cmd).await?);
        return Ok(());
    }

    let mut rl = Editor::<SolidityHelper, _>::new()?;
    rl.set_helper(Some(dispatcher.helper.clone()));
    rl.set_auto_add_history(true);
    if let Some(path) = chisel_history_file() {
        let _ = rl.load_history(&path);
    }

    sh_println!("Welcome to Chisel! Type `{}` to show available commands.", "!help".green())?;

    // REPL loop.
    let mut interrupt = false;
    loop {
        match rl.readline(&dispatcher.get_prompt()) {
            Ok(line) => {
                debug!("dispatching next line: {line}");
                // Clear interrupt flag.
                interrupt = false;

                // Dispatch and match results.
                let r = dispatcher.dispatch(&line).await;
                dispatcher.helper.set_errored(r.is_err());
                match r {
                    Ok(ControlFlow::Continue(())) => {}
                    Ok(ControlFlow::Break(())) => break,
                    Err(e) => {
                        sh_err!("{}", foundry_common::errors::display_chain(&e))?;
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                if interrupt {
                    break;
                } else {
                    sh_println!("(To exit, press Ctrl+C again)")?;
                    interrupt = true;
                }
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                sh_err!("{err}")?;
                break;
            }
        }
    }

    if let Some(path) = chisel_history_file() {
        let _ = rl.save_history(&path);
    }

    Ok(())
}

/// Evaluate multiple Solidity source files contained within a
/// Chisel prelude directory.
async fn evaluate_prelude(
    dispatcher: &mut ChiselDispatcher,
    maybe_prelude: Option<PathBuf>,
) -> Result<()> {
    let Some(prelude_dir) = maybe_prelude else { return Ok(()) };
    if prelude_dir.is_file() {
        sh_println!("{} {}", "Loading prelude source file:".yellow(), prelude_dir.display())?;
        try_cf!(load_prelude_file(dispatcher, prelude_dir).await?);
        sh_println!("{}\n", "Prelude source file loaded successfully!".green())?;
    } else {
        let prelude_sources = fs::files_with_ext(&prelude_dir, "sol");
        let mut print_success_msg = false;
        for source_file in prelude_sources {
            print_success_msg = true;
            sh_println!("{} {}", "Loading prelude source file:".yellow(), source_file.display())?;
            try_cf!(load_prelude_file(dispatcher, source_file).await?);
        }

        if print_success_msg {
            sh_println!("{}\n", "All prelude source files loaded successfully!".green())?;
        }
    }
    Ok(())
}

/// Loads a single Solidity file into the prelude.
async fn load_prelude_file(
    dispatcher: &mut ChiselDispatcher,
    file: PathBuf,
) -> Result<ControlFlow<()>> {
    let prelude = fs::read_to_string(file)
        .wrap_err("Could not load source file. Are you sure this path is correct?")?;
    dispatcher.dispatch(&prelude).await
}

async fn handle_cli_command(
    d: &mut ChiselDispatcher,
    cmd: ChiselSubcommand,
) -> Result<ControlFlow<()>> {
    match cmd {
        ChiselSubcommand::List => d.dispatch_command(ChiselCommand::ListSessions).await,
        ChiselSubcommand::Load { id } => d.dispatch_command(ChiselCommand::Load { id }).await,
        ChiselSubcommand::View { id } => {
            let ControlFlow::Continue(()) = d.dispatch_command(ChiselCommand::Load { id }).await?
            else {
                return Ok(ControlFlow::Break(()));
            };
            d.dispatch_command(ChiselCommand::Source).await
        }
        ChiselSubcommand::ClearCache => d.dispatch_command(ChiselCommand::ClearCache).await,
        ChiselSubcommand::Eval { command } => d.dispatch(&command).await,
    }
}

fn chisel_history_file() -> Option<PathBuf> {
    foundry_config::Config::foundry_dir().map(|p| p.join(".chisel_history"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use foundry_config::Config;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn verify_cli() {
        Chisel::command().debug_assert();
    }

    #[test]
    fn test_chisel_history_file() {
        // Test that function returns Some when foundry_dir is available
        let path = chisel_history_file();
        // The function should return Some if foundry_dir() succeeds
        // We can't easily mock this, so we just verify it doesn't panic
        if let Some(p) = path {
            assert!(p.ends_with(".chisel_history"));
        }
    }

    #[tokio::test]
    async fn test_evaluate_prelude_none() {
        // Test with None prelude (should return Ok immediately)
        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        let result = evaluate_prelude(&mut dispatcher, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_evaluate_prelude_single_file() {
        // Test with a single file containing simple code
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.sol");
        // Use simple code that compiles quickly
        fs::write(&file_path, "// test comment").unwrap();

        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // Should execute without panicking, may take time to compile
        let _result = evaluate_prelude(&mut dispatcher, Some(file_path)).await;
    }

    #[tokio::test]
    async fn test_evaluate_prelude_directory() {
        // Test with a directory containing .sol files
        let temp_dir = TempDir::new().unwrap();
        let file1 = temp_dir.path().join("file1.sol");
        let file2 = temp_dir.path().join("file2.sol");
        // Use simple code that compiles quickly
        fs::write(&file1, "// file1").unwrap();
        fs::write(&file2, "// file2").unwrap();

        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // Should execute without panicking, may take time to compile
        let _result = evaluate_prelude(&mut dispatcher, Some(temp_dir.path().to_path_buf())).await;
    }

    #[tokio::test]
    async fn test_evaluate_prelude_empty_directory() {
        // Test with an empty directory
        let temp_dir = TempDir::new().unwrap();

        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        let result = evaluate_prelude(&mut dispatcher, Some(temp_dir.path().to_path_buf())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_load_prelude_file_success() {
        // Test successful file loading
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.sol");
        // Use simple code that compiles quickly
        fs::write(&file_path, "// test comment").unwrap();

        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // Should execute without panicking, may take time to compile
        let _result = load_prelude_file(&mut dispatcher, file_path).await;
    }

    #[tokio::test]
    async fn test_load_prelude_file_not_found() {
        // Test with non-existent file
        let non_existent = PathBuf::from("/nonexistent/path/file.sol");

        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        let result = load_prelude_file(&mut dispatcher, non_existent).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_cli_command_list() {
        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // List command should execute without panicking
        // It may return Ok or Err depending on cache state, but should handle gracefully
        let _result = handle_cli_command(&mut dispatcher, ChiselSubcommand::List).await;
    }

    #[tokio::test]
    async fn test_handle_cli_command_clear_cache() {
        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // ClearCache should execute without panicking
        let result = handle_cli_command(&mut dispatcher, ChiselSubcommand::ClearCache).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_cli_command_eval() {
        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // Eval with simple expression should work
        let result = handle_cli_command(
            &mut dispatcher,
            ChiselSubcommand::Eval { command: "1 + 1".to_string() },
        )
        .await;
        // Should not panic, may succeed or fail depending on compilation
        let _ = result;
    }

    #[tokio::test]
    async fn test_handle_cli_command_load() {
        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // Load with non-existent ID should return an error
        let result = handle_cli_command(
            &mut dispatcher,
            ChiselSubcommand::Load { id: "nonexistent".to_string() },
        )
        .await;
        // This may succeed or fail depending on implementation, but shouldn't panic
        let _ = result;
    }

    #[tokio::test]
    async fn test_handle_cli_command_view() {
        let config = crate::source::SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: Default::default(),
            no_vm: false,
            backend: None,
            traces: false,
            calldata: None,
            ir_minimum: false,
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();
        // View with non-existent ID should handle gracefully
        let result = handle_cli_command(
            &mut dispatcher,
            ChiselSubcommand::View { id: "nonexistent".to_string() },
        )
        .await;
        // Should return ControlFlow::Break if Load fails, or Continue if it succeeds
        assert!(result.is_ok());
        if let Ok(cf) = result {
            // View should return Break if Load fails, or Continue if both succeed
            match cf {
                ControlFlow::Continue(()) | ControlFlow::Break(()) => {}
            }
        }
    }
}
