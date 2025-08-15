use clap_complete::{Shell as ClapCompleteShell, aot::Generator};
use clap_complete_nushell::Nushell;

#[derive(Clone, Copy)]
pub enum Shell {
    ClapCompeleteShell(ClapCompleteShell),
    Nushell,
}

impl clap::ValueEnum for Shell {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::ClapCompeleteShell(ClapCompleteShell::Bash),
            Self::ClapCompeleteShell(ClapCompleteShell::Zsh),
            Self::ClapCompeleteShell(ClapCompleteShell::Fish),
            Self::ClapCompeleteShell(ClapCompleteShell::PowerShell),
            Self::ClapCompeleteShell(ClapCompleteShell::Elvish),
            Self::Nushell,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::ClapCompeleteShell(shell) => shell.to_possible_value(),
            Self::Nushell => Some(clap::builder::PossibleValue::new("nushell")),
        }
    }
}

impl Generator for Shell {
    fn file_name(&self, name: &str) -> String {
        match self {
            Self::ClapCompeleteShell(shell) => shell.file_name(name),
            Self::Nushell => Nushell.file_name(name),
        }
    }

    fn generate(&self, cmd: &clap::Command, buf: &mut dyn std::io::Write) {
        match self {
            Self::ClapCompeleteShell(shell) => shell.generate(cmd, buf),
            Self::Nushell => Nushell.generate(cmd, buf),
        }
    }
}
