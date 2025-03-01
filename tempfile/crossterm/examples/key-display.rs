//! Demonstrates the display format of key events.
//!
//! This example demonstrates the display format of key events, which is useful for displaying in
//! the help section of a terminal application.
//!
//! cargo run --example key-display

use std::io;

use crossterm::event::{KeyEventKind, KeyModifiers};
use crossterm::{
    event::{read, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};

const HELP: &str = r#"Key display
 - Press any key to see its display format
 - Use Esc to quit
"#;

fn main() -> io::Result<()> {
    println!("{}", HELP);
    enable_raw_mode()?;
    if let Err(e) = print_events() {
        println!("Error: {:?}\r", e);
    }
    disable_raw_mode()?;
    Ok(())
}

fn print_events() -> io::Result<()> {
    loop {
        let event = read()?;
        match event {
            Event::Key(event) if event.kind == KeyEventKind::Press => {
                print!("Key pressed: ");
                if event.modifiers != KeyModifiers::NONE {
                    print!("{}+", event.modifiers);
                }
                println!("{}\r", event.code);
                if event.code == KeyCode::Esc {
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
