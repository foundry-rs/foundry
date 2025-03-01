use std::io;

use console::{Key, Term};

fn main() -> io::Result<()> {
    let term = Term::stdout();
    term.write_line("Press any key. Esc to exit")?;
    loop {
        let key = term.read_key()?;
        term.write_line(&format!("You pressed {:?}", key))?;
        if key == Key::Escape {
            break;
        }
    }
    Ok(())
}
