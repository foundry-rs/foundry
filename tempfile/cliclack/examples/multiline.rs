use std::io;

use cliclack::{clear_screen, intro, outro, outro_cancel, spinner};
use console::{style, Key, Term};

fn main() -> std::io::Result<()> {
    ctrlc::set_handler(move || {}).expect("setting Ctrl-C handler");

    clear_screen()?;
    intro(style(" multiline support ").on_cyan().black())?;

    let path: String = cliclack::input("Where should we create your project?\n👇")
        .placeholder("./sparkling-solid")
        .interact()?;

    let _password = cliclack::password("Provide a password\n🔒")
        .mask('▪')
        .interact()?;

    let _kind = cliclack::select(format!("Pick a project type within '{path}'\n💪"))
        .initial_value("ts")
        .item("ts", "TypeScript", "")
        .item("js", "JavaScript", "")
        .item("coffee", "CoffeeScript", "oh no")
        .interact()?;

    let _tools = cliclack::multiselect("Select additional tools\n🛠️")
        .initial_values(vec!["prettier", "eslint"])
        .item("prettier", "Prettier", "recommended")
        .item("eslint", "ESLint", "recommended")
        .item("stylelint", "Stylelint", "")
        .item("gh-action", "GitHub Action", "")
        .interact()?;

    let spinner = spinner();
    let message = format!(
        "{}\n{}\n\n{}",
        style("Installation").bold(),
        style("Press Esc, Enter, or Ctrl-C").dim(),
        style("Check it out, we're multilining!").magenta().italic()
    );
    spinner.start(message);

    let term = Term::stderr();
    loop {
        match term.read_key() {
            Ok(Key::Escape) => {
                spinner.cancel("Installation");
                outro_cancel("Cancelled")?;
            }
            Ok(Key::Enter) => {
                spinner.stop("Installation");
                outro("Done!")?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                spinner.error("Installation");
                outro_cancel("Interrupted")?;
            }
            _ => continue,
        }
        break;
    }

    Ok(())
}
