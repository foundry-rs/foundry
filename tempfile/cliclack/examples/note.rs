use cliclack::{intro, log, outro_note};
use console::style;

fn main() -> std::io::Result<()> {
    intro(style(" note").on_cyan().black())?;
    log::step("This is a submitted step")?;
    cliclack::note("This is an inline note", "This is a note message")?;
    log::warning("Watch out, the next one is an outro!")?;
    outro_note("This is an outro note", "Some explanation")?;

    Ok(())
}
