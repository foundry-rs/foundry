use std::{io::IsTerminal, path::PathBuf};

use eyre::{Context as _, ContextCompat as _};
use foundry_common::{io::style::WARN, stdin::parse_line, Shell};
use foundry_config::network_family::NetworkFamily;

/// Loads config and checks if there is a binary remapping for the current binary.
/// If there is a remapping, returns the path to the binary that should be executed.
/// Returns `None` if the binary is not remapped _or_ if the current binary is not found in the
/// config.
///
/// Required user consent for the redirect. If consent is not provided (either via
/// config or prompt), will return an error.
pub fn should_redirect_to() -> eyre::Result<Option<PathBuf>> {
    let current_exe =
        std::env::current_exe().context("Unable to query the current executable name")?;
    let binary_name = current_exe
        .file_stem()
        .with_context(|| "Unable to parse executable file name")?
        .to_str()
        .context("Executable name is not UTF-8")?;
    let config = foundry_config::Config::load()?;
    let Some(redirect) = config.binary_mappings().redirect_for(binary_name).cloned() else {
        trace!(
            binary_name = ?binary_name,
            redirects = ?config.binary_mappings(),
            "No redirect is found",
        );
        return Ok(None)
    };

    // We cannot use shell macros, since they will implicitly initialize global shell.
    let mut shell = Shell::default();

    // Ensure that if redirect exists, user opted in to it.
    let redirect = match config.allow_alternative_binaries {
        Some(true) => {
            // User opted in to alternative binaries.
            Some(redirect)
        }
        Some(false) => {
            // User opted out of alternative binaries.
            shell.warn("A binary remapping was detected, but `allow_alternative_binaries` is set to false, which prohibits the redirects.")?;
            eyre::bail!("Binary remapping is not allowed by the user.");
        }
        None => {
            // Prompt user to allow alternative binary.
            shell.warn("")?;
            let mut lines = vec![
                "A binary remapping was detected, but `allow_alternative_binaries` is not set in the config.".to_string(),
                "You can set `allow_alternative_binaries` config to `true` to avoid this prompt.".to_string(),
                "Foundry team is not responsible for the safety of the redirected binary.".to_string(),
                format!("If you would allow it, the execution would be redirected to the following binary: {redirect:?}")
            ];
            append_attestation_docs(&mut lines, &config, &redirect);

            print_box_message(&mut shell, &lines)?;

            let std = std::io::stdin();
            if !std.is_terminal() {
                shell.error("std is not a terminal, cannot prompt user. Ignoring the redirect")?;
                eyre::bail!("Binary remapping must be explicitly allowed");
            }

            shell.print_out("Do you want to allow the redirect? [y/N] ")?;
            std::io::Write::flush(&mut std::io::stdout())?;

            let response: String = parse_line()?;
            if matches!(response.as_str(), "y" | "Y") {
                Some(redirect)
            } else {
                eyre::bail!("User did not allow redirecting to another binary");
            }
        }
    };
    Ok(redirect)
}

/// Appends the lines that explain how to verify the binary attestation.
fn append_attestation_docs(
    lines: &mut Vec<String>,
    config: &foundry_config::Config,
    binary_name: &PathBuf,
) {
    if config.network_family == NetworkFamily::Zksync {
        lines.extend_from_slice(&[
            String::new(),
            "Tip:".to_string(),
            "ZKsync network family is selected in the config.".to_string(),
            "To verify the authenticity of the binary, you can use the following command (Linux/MacOS):".to_string(),
            format!("$ gh attestation verify --owner matter-labs $(which {binary_name:?})"),
        ])
    }
}

fn print_box_message(shell: &mut Shell, lines: &[String]) -> eyre::Result<()> {
    // Print messages via `shell.print` rounding them with an ascii box.
    let max_len = lines.iter().map(String::len).max().unwrap_or(0);
    let top = format!("+{:-<1$}+\n", "", max_len + 2);
    shell.write_stdout(&top, &WARN)?;
    for line in lines {
        shell.write_stdout("| ", &WARN)?;
        shell.print_out(format!("{line:<max_len$}"))?;
        shell.write_stdout(" |\n", &WARN)?;
    }
    shell.write_stdout(&top, &WARN)?;
    Ok(())
}

/// Launches the `to` binary with the same arguments as the current binary.
/// E.g. if user runs `forge build --arg1 --arg2`, and `to` is `/path/to/custom/forge`, then
/// this function will run `/path/to/custom/forge build --arg1 --arg2`.
pub fn redirect_execution(to: PathBuf) -> eyre::Result<()> {
    // We cannot use shell macros, since they will implicitly initialize global shell.
    let mut shell = Shell::default();
    shell.warn(format!("Redirecting execution to: {to:?}"))?;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let status = std::process::Command::new(to)
        .args(args)
        .status()
        .map_err(|e| eyre::eyre!("Failed to run command: {}", e))?;
    if !status.success() {
        eyre::bail!("Command failed with status: {}", status);
    }
    Ok(())
}
