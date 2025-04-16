// TODO(dani): tmp for testing

#![allow(dead_code, clippy::disallowed_macros)]

use std::{io::Read, path::PathBuf};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let (src, path) = if args.len() < 2 || args[1] == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).unwrap();
        (s, None)
    } else {
        let path = PathBuf::from(&args[1]);
        (std::fs::read_to_string(&path).unwrap(), Some(path))
    };
    let config = foundry_config::Config::load().unwrap();
    match forge_fmt_2::format_source(&src, path.as_deref(), config.fmt) {
        Ok(formatted) => {
            print!("{formatted}");
        }
        Err(e) => {
            eprintln!("failed formatting: {e}");
            std::process::exit(1);
        }
    }
}
