use colored::*;
use std::fmt;

pub fn success(line: impl AsRef<str>) {
    println!("{}", line.as_ref().bright_green());
}

pub fn info(line: impl AsRef<str>) {
    println!("{}", line.as_ref().bright_blue());
}

pub fn debug(line: impl AsRef<str>) {
    println!("{}", line.as_ref().bright_cyan().dimmed());
}

pub fn warn(line: impl AsRef<str>) {
    eprintln!("{}", line.as_ref().yellow());
}

pub fn error(line: impl AsRef<str>) {
    eprintln!("{}", line.as_ref().bright_red());
}

#[derive(Debug, Default)]
pub struct Prompt {
    /// The current set project
    pub project: Option<String>,
}

impl Prompt {
    pub fn new(project: String) -> Prompt {
        Prompt { project: Some(project) }
    }
}

impl fmt::Display for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("[forge]")?;
        if let Some(ref project) = self.project {
            write!(f, "[{}]", project)?;
        }
        write!(f, " > ")
    }
}

pub fn print_banner() {
    println!(
        r#"
       ___
     /'___)
    | (__  _    _ __  __     __
    | ,__)'_`\ ( '__)'_ `\ /'__`\
    | | ( (_) )| | ( (_) |(  ___/
    (_) `\___/'(_) `\__  |`\____)
                   ( )_) |
                    \___/'
        {} | {}
      {}
"#,
        "solidity".green(),
        "shell".green(),
        "https://github.com/gakonst/foundry".green()
    );
}
