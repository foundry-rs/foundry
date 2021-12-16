use colored::*;

#[inline]
fn help(cmd: &str, descr: &str) {
    println!("    {:13} {}", cmd, descr);
}

pub fn print_help() {
    println!("{}", "COMMANDS".bright_yellow());
    help("list", "lists various information");
    println!("\nRun <command> -h for more help.\n");
}
