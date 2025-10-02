use clap_complete::{Shell as ClapCompleteShell, aot::Generator};
use clap_complete_nushell::Nushell;

#[derive(Clone, Copy)]
pub enum Shell {
    ClapCompleteShell(ClapCompleteShell),
    Nushell,
}

impl clap::ValueEnum for Shell {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::ClapCompleteShell(ClapCompleteShell::Bash),
            Self::ClapCompleteShell(ClapCompleteShell::Zsh),
            Self::ClapCompleteShell(ClapCompleteShell::Fish),
            Self::ClapCompleteShell(ClapCompleteShell::PowerShell),
            Self::ClapCompleteShell(ClapCompleteShell::Elvish),
            Self::Nushell,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::ClapCompleteShell(shell) => shell.to_possible_value(),
            Self::Nushell => Some(clap::builder::PossibleValue::new("nushell")),
        }
    }
}

impl Generator for Shell {
    fn file_name(&self, name: &str) -> String {
        match self {
            Self::ClapCompleteShell(shell) => shell.file_name(name),
            Self::Nushell => Nushell.file_name(name),
        }
    }

    fn generate(&self, cmd: &clap::Command, buf: &mut dyn std::io::Write) {
        match self {
            Self::ClapCompleteShell(shell) => shell.generate(cmd, buf),
            Self::Nushell => Nushell.generate(cmd, buf),
        }
    }
}
