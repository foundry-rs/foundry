use cliclack::{intro, log, outro_cancel};
use console::style;

fn main() -> std::io::Result<()> {
    intro(style(" log ").on_cyan().black())?;
    log::remark("This is a simple message")?;
    log::warning("This is a warning")?;
    log::error("This is an error")?;
    log::success("This is a success")?;
    log::info("This is an info")?;
    log::step("This is a submitted step")?;
    outro_cancel("Like it's cancelled")?;

    Ok(())
}
