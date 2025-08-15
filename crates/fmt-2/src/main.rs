// TODO(dani): tmp for testing

#![allow(dead_code, clippy::disallowed_macros)]

use std::{io::Read, path::PathBuf};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

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
    let result = forge_fmt_2::format_source(&src, path.as_deref(), config.fmt);
    if let Some(formatted) = result.ok_ref() {
        print!("{formatted}");
    }
    if let Some(diagnostics) = result.err_ref() {
        if result.is_err() {
            eprintln!("failed formatting:\n{diagnostics}");
            std::process::exit(1);
        } else {
            eprintln!("formatted with output:\n{diagnostics}");
        }
    }
}
