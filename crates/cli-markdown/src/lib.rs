//! Generate Markdown documentation for clap command-line tools.
//!
//! This is a fork of [`clap-markdown`](https://crates.io/crates/clap-markdown) with the following
//! enhancements:
//! - Support for grouped options by help heading ([PR #48](https://github.com/ConnorGray/clap-markdown/pull/48))
//! - Show environment variable names for arguments ([PR #50](https://github.com/ConnorGray/clap-markdown/pull/50))
//! - Add version information to generated Markdown ([PR #52](https://github.com/ConnorGray/clap-markdown/pull/52))

use std::{
    collections::BTreeMap,
    fmt::{self, Write},
};

use clap::builder::PossibleValue;

/// Options to customize the structure of the output Markdown document.
#[non_exhaustive]
pub struct MarkdownOptions {
    title: Option<String>,
    show_footer: bool,
    show_table_of_contents: bool,
    show_aliases: bool,
}

impl MarkdownOptions {
    /// Construct a default instance of `MarkdownOptions`.
    pub fn new() -> Self {
        Self { title: None, show_footer: true, show_table_of_contents: true, show_aliases: true }
    }

    /// Set a custom title to use in the generated document.
    pub fn title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    /// Whether to show the default footer advertising `clap-markdown`.
    pub fn show_footer(mut self, show: bool) -> Self {
        self.show_footer = show;
        self
    }

    /// Whether to show the default table of contents.
    pub fn show_table_of_contents(mut self, show: bool) -> Self {
        self.show_table_of_contents = show;
        self
    }

    /// Whether to show aliases for arguments and commands.
    pub fn show_aliases(mut self, show: bool) -> Self {
        self.show_aliases = show;
        self
    }
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Format the help information for `command` as Markdown.
pub fn help_markdown<C: clap::CommandFactory>() -> String {
    let command = C::command();
    help_markdown_command(&command)
}

/// Format the help information for `command` as Markdown, with custom options.
pub fn help_markdown_custom<C: clap::CommandFactory>(options: &MarkdownOptions) -> String {
    let command = C::command();
    help_markdown_command_custom(&command, options)
}

/// Format the help information for `command` as Markdown.
pub fn help_markdown_command(command: &clap::Command) -> String {
    help_markdown_command_custom(command, &Default::default())
}

/// Format the help information for `command` as Markdown, with custom options.
pub fn help_markdown_command_custom(command: &clap::Command, options: &MarkdownOptions) -> String {
    let mut buffer = String::with_capacity(100);
    write_help_markdown(&mut buffer, command, options);
    buffer
}

/// Format the help information for `command` as Markdown and print it.
///
/// Output is printed to the standard output.
#[allow(clippy::disallowed_macros)]
pub fn print_help_markdown<C: clap::CommandFactory>() {
    let command = C::command();
    let mut buffer = String::with_capacity(100);
    write_help_markdown(&mut buffer, &command, &Default::default());
    println!("{buffer}");
}

fn write_help_markdown(buffer: &mut String, command: &clap::Command, options: &MarkdownOptions) {
    let title_name = get_canonical_name(command);

    let title = match options.title {
        Some(ref title) => title.to_owned(),
        None => format!("Command-Line Help for `{title_name}`"),
    };
    writeln!(buffer, "# {title}\n",).unwrap();

    writeln!(
        buffer,
        "This document contains the help content for the `{title_name}` command-line program.\n",
    )
    .unwrap();

    // Write the version if available (PR #52)
    if let Some(version) = command.get_version() {
        let version_str = version.to_string();

        if version_str.contains('\n') {
            // Multi-line version: use a code block
            writeln!(buffer, "**Version:**\n\n```\n{}\n```\n", version_str.trim()).unwrap();
        } else {
            // Single-line version: use inline code
            writeln!(buffer, "**Version:** `{version_str}`\n").unwrap();
        }
    }

    // Write the table of contents
    if options.show_table_of_contents {
        writeln!(buffer, "**Command Overview:**\n").unwrap();
        build_table_of_contents_markdown(buffer, Vec::new(), command, 0).unwrap();
        writeln!(buffer).unwrap();
    }

    // Write the commands/subcommands sections
    build_command_markdown(buffer, Vec::new(), command, 0, options).unwrap();

    // Write the footer
    if options.show_footer {
        write!(
            buffer,
            r#"<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
"#
        )
        .unwrap();
    }
}

fn build_table_of_contents_markdown(
    buffer: &mut String,
    parent_command_path: Vec<String>,
    command: &clap::Command,
    _depth: usize,
) -> std::fmt::Result {
    // Don't document commands marked with `clap(hide = true)`
    if command.is_hide_set() {
        return Ok(());
    }

    let title_name = get_canonical_name(command);

    let command_path = {
        let mut command_path = parent_command_path;
        command_path.push(title_name);
        command_path
    };

    writeln!(buffer, "* [`{}`↴](#{})", command_path.join(" "), command_path.join("-"),)?;

    for subcommand in command.get_subcommands() {
        build_table_of_contents_markdown(buffer, command_path.clone(), subcommand, _depth + 1)?;
    }

    Ok(())
}

fn build_command_markdown(
    buffer: &mut String,
    parent_command_path: Vec<String>,
    command: &clap::Command,
    _depth: usize,
    options: &MarkdownOptions,
) -> std::fmt::Result {
    // Don't document commands marked with `clap(hide = true)`
    if command.is_hide_set() {
        return Ok(());
    }

    let title_name = get_canonical_name(command);

    let command_path = {
        let mut command_path = parent_command_path.clone();
        command_path.push(title_name);
        command_path
    };

    // Write the markdown heading
    writeln!(buffer, "## `{}`\n", command_path.join(" "))?;

    if let Some(long_about) = command.get_long_about() {
        writeln!(buffer, "{long_about}\n")?;
    } else if let Some(about) = command.get_about() {
        writeln!(buffer, "{about}\n")?;
    }

    if let Some(help) = command.get_before_long_help() {
        writeln!(buffer, "{help}\n")?;
    } else if let Some(help) = command.get_before_help() {
        writeln!(buffer, "{help}\n")?;
    }

    writeln!(
        buffer,
        "**Usage:** `{}{}`\n",
        if parent_command_path.is_empty() {
            String::new()
        } else {
            let mut s = parent_command_path.join(" ");
            s.push(' ');
            s
        },
        command.clone().render_usage().to_string().replace("Usage: ", "")
    )?;

    if options.show_aliases {
        let aliases = command.get_visible_aliases().collect::<Vec<&str>>();
        if let Some(aliases_str) = get_alias_string(&aliases) {
            writeln!(
                buffer,
                "**{}:** {aliases_str}\n",
                pluralize(aliases.len(), "Command Alias", "Command Aliases")
            )?;
        }
    }

    if let Some(help) = command.get_after_long_help() {
        writeln!(buffer, "{help}\n")?;
    } else if let Some(help) = command.get_after_help() {
        writeln!(buffer, "{help}\n")?;
    }

    // Subcommands
    if command.get_subcommands().next().is_some() {
        writeln!(buffer, "###### **Subcommands:**\n")?;

        for subcommand in command.get_subcommands() {
            if subcommand.is_hide_set() {
                continue;
            }

            let title_name = get_canonical_name(subcommand);
            let about = match subcommand.get_about() {
                Some(about) => about.to_string(),
                None => String::new(),
            };

            writeln!(buffer, "* `{title_name}` — {about}",)?;
        }

        writeln!(buffer)?;
    }

    // Arguments (positional)
    if command.get_positionals().next().is_some() {
        writeln!(buffer, "###### **Arguments:**\n")?;

        for pos_arg in command.get_positionals() {
            write_arg_markdown(buffer, pos_arg)?;
        }

        writeln!(buffer)?;
    }

    // Options (grouped by help heading) - PR #48
    let non_pos: Vec<_> =
        command.get_arguments().filter(|arg| !arg.is_positional() && !arg.is_hide_set()).collect();

    if !non_pos.is_empty() {
        // Group arguments by help heading
        let mut grouped_args: BTreeMap<&str, Vec<&clap::Arg>> = BTreeMap::new();

        for arg in non_pos {
            let heading = arg.get_help_heading().unwrap_or("Options");
            grouped_args.entry(heading).or_default().push(arg);
        }

        // Write each group with its heading
        for (heading, args) in grouped_args {
            writeln!(buffer, "###### **{heading}:**\n")?;

            for arg in args {
                write_arg_markdown(buffer, arg)?;
            }

            writeln!(buffer)?;
        }
    }

    // Include extra space between commands
    write!(buffer, "\n\n")?;

    for subcommand in command.get_subcommands() {
        build_command_markdown(buffer, command_path.clone(), subcommand, _depth + 1, options)?;
    }

    Ok(())
}

fn write_arg_markdown(buffer: &mut String, arg: &clap::Arg) -> fmt::Result {
    // Markdown list item
    write!(buffer, "* ")?;

    let value_name: String = match arg.get_value_names() {
        Some([name, ..]) => name.as_str().to_owned(),
        Some([]) => unreachable!("clap Arg::get_value_names() returned Some(..) of empty list"),
        None => arg.get_id().to_string().to_ascii_uppercase(),
    };

    match (arg.get_short(), arg.get_long()) {
        (Some(short), Some(long)) => {
            if arg.get_action().takes_values() {
                write!(buffer, "`-{short}`, `--{long} <{value_name}>`")?
            } else {
                write!(buffer, "`-{short}`, `--{long}`")?
            }
        }
        (Some(short), None) => {
            if arg.get_action().takes_values() {
                write!(buffer, "`-{short} <{value_name}>`")?
            } else {
                write!(buffer, "`-{short}`")?
            }
        }
        (None, Some(long)) => {
            if arg.get_action().takes_values() {
                write!(buffer, "`--{long} <{value_name}>`")?
            } else {
                write!(buffer, "`--{long}`")?
            }
        }
        (None, None) => {
            debug_assert!(
                arg.is_positional(),
                "unexpected non-positional Arg with neither short nor long name: {arg:?}"
            );
            write!(buffer, "`<{value_name}>`",)?;
        }
    }

    if let Some(aliases) = arg.get_visible_aliases().as_deref()
        && let Some(aliases_str) = get_alias_string(aliases)
    {
        write!(buffer, " [{}: {aliases_str}]", pluralize(aliases.len(), "alias", "aliases"))?;
    }

    if let Some(help) = arg.get_long_help() {
        buffer.push_str(&indent(&help.to_string(), " — ", "   "))
    } else if let Some(short_help) = arg.get_help() {
        writeln!(buffer, " — {short_help}")?;
    } else {
        writeln!(buffer)?;
    }

    // Arg default values
    if !arg.get_default_values().is_empty() {
        let default_values: String = arg
            .get_default_values()
            .iter()
            .map(|value| format!("`{}`", value.to_string_lossy()))
            .collect::<Vec<String>>()
            .join(", ");

        if arg.get_default_values().len() > 1 {
            writeln!(buffer, "\n  Default values: {default_values}")?;
        } else {
            writeln!(buffer, "\n  Default value: {default_values}")?;
        }
    }

    // Arg possible values
    let possible_values: Vec<PossibleValue> =
        arg.get_possible_values().into_iter().filter(|pv| !pv.is_hide_set()).collect();

    if !possible_values.is_empty() && !matches!(arg.get_action(), clap::ArgAction::SetTrue) {
        let any_have_help: bool = possible_values.iter().any(|pv| pv.get_help().is_some());

        if any_have_help {
            let text: String = possible_values
                .iter()
                .map(|pv| match pv.get_help() {
                    Some(help) => {
                        format!("  - `{}`:\n    {}\n", pv.get_name(), help)
                    }
                    None => format!("  - `{}`\n", pv.get_name()),
                })
                .collect::<Vec<String>>()
                .join("");

            writeln!(buffer, "\n  Possible values:\n{text}")?;
        } else {
            let text: String = possible_values
                .iter()
                .map(|pv| format!("`{}`", pv.get_name()))
                .collect::<Vec<String>>()
                .join(", ");

            writeln!(buffer, "\n  Possible values: {text}\n")?;
        }
    }

    // Arg environment variable (PR #50)
    if !arg.is_hide_env_set()
        && let Some(env) = arg.get_env()
    {
        writeln!(buffer, "\n  Environment variable: `{}`", env.to_string_lossy())?;
    }

    Ok(())
}

/// Utility function to get the canonical name of a command.
fn get_canonical_name(command: &clap::Command) -> String {
    command
        .get_display_name()
        .or_else(|| command.get_bin_name())
        .map(|name| name.to_owned())
        .unwrap_or_else(|| command.get_name().to_owned())
}

/// Indents non-empty lines. The output always ends with a newline.
fn indent(s: &str, first: &str, rest: &str) -> String {
    if s.is_empty() {
        return "\n".to_string();
    }
    let mut result = String::new();
    let mut first_line = true;

    for line in s.lines() {
        if !line.is_empty() {
            result.push_str(if first_line { first } else { rest });
            result.push_str(line);
            first_line = false;
        }
        result.push('\n');
    }
    result
}

fn get_alias_string(aliases: &[&str]) -> Option<String> {
    if aliases.is_empty() {
        return None;
    }

    Some(aliases.iter().map(|alias| format!("`{alias}`")).collect::<Vec<_>>().join(", "))
}

fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Arg, Command};
    use pretty_assertions::assert_eq;

    #[test]
    fn test_indent() {
        assert_eq!(&indent("Header\n\nMore info", "___", "~~~~"), "___Header\n\n~~~~More info\n");
        assert_eq!(
            &indent("Header\n\nMore info\n", "___", "~~~~"),
            &indent("Header\n\nMore info", "___", "~~~~"),
        );
        assert_eq!(&indent("", "___", "~~~~"), "\n");
        assert_eq!(&indent("\n", "___", "~~~~"), "\n");
    }

    #[test]
    fn test_version_output() {
        let app = Command::new("test-app").version("1.2.3").about("A test application");

        let markdown =
            help_markdown_command_custom(&app, &MarkdownOptions::new().show_footer(false));

        assert!(markdown.contains("**Version:** `1.2.3`"), "Should contain version");
    }

    #[test]
    fn test_multiline_version() {
        let multi_line_version = "my-cli 1.2.3 (abc123)\nmy-lib 2.0.0 (789xyz)";

        let app = Command::new("my-cli").version(multi_line_version).about("Multi-version CLI");

        let markdown =
            help_markdown_command_custom(&app, &MarkdownOptions::new().show_footer(false));

        assert!(markdown.contains("**Version:**\n\n```"), "Should use code block for multi-line");
    }

    #[test]
    fn test_env_var_output() {
        let app = Command::new("env-test").about("Test env var output").arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .env("CONFIG_PATH")
                .help("Path to config file"),
        );

        let markdown =
            help_markdown_command_custom(&app, &MarkdownOptions::new().show_footer(false));

        assert!(
            markdown.contains("Environment variable: `CONFIG_PATH`"),
            "Should show env var. Output: {markdown}"
        );
    }

    #[test]
    fn test_grouped_options() {
        let app = Command::new("grouped-app")
            .about("Test app with grouped options")
            .arg(
                Arg::new("verbose")
                    .short('v')
                    .long("verbose")
                    .help("Enable verbose output")
                    .help_heading("General Options")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("input")
                    .short('i')
                    .long("input")
                    .help("Input file")
                    .help_heading("File Options")
                    .value_name("FILE"),
            )
            .arg(
                Arg::new("format")
                    .short('f')
                    .long("format")
                    .help("Output format")
                    .value_name("FORMAT"),
            );

        let markdown =
            help_markdown_command_custom(&app, &MarkdownOptions::new().show_footer(false));

        assert!(markdown.contains("###### **File Options:**"), "Should have File Options heading");
        assert!(
            markdown.contains("###### **General Options:**"),
            "Should have General Options heading"
        );
        assert!(markdown.contains("###### **Options:**"), "Should have default Options heading");
    }

    #[test]
    fn test_no_grouped_options_backward_compatibility() {
        let app = Command::new("simple-app")
            .about("Test app without grouped options")
            .arg(
                Arg::new("verbose")
                    .short('v')
                    .long("verbose")
                    .help("Enable verbose output")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("output").short('o').long("output").help("Output file").value_name("FILE"),
            );

        let markdown =
            help_markdown_command_custom(&app, &MarkdownOptions::new().show_footer(false));

        assert!(markdown.contains("###### **Options:**"), "Should have default Options heading");
        assert!(markdown.contains("`-v`, `--verbose`"), "Should have verbose option");
        assert!(markdown.contains("`-o`, `--output <FILE>`"), "Should have output option");
    }
}
