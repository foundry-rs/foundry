use std::collections::BTreeMap;

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
            Self::ClapCompleteShell(ClapCompleteShell::Fish) => {
                let mut completion = Vec::new();
                ClapCompleteShell::Fish.generate(cmd, &mut completion);
                let completion = compact_fish_completion(cmd.get_name(), completion);
                buf.write_all(completion.as_bytes()).expect("failed to write completion file");
            }
            Self::ClapCompleteShell(shell) => shell.generate(cmd, buf),
            Self::Nushell => Nushell.generate(cmd, buf),
        }
    }
}

fn compact_fish_completion(cmd_name: &str, completion: Vec<u8>) -> String {
    let completion =
        String::from_utf8(completion).expect("fish completion scripts should be UTF-8");

    let mut counts = BTreeMap::<String, usize>::new();
    for line in completion.lines() {
        if let Some(condition) = fish_complete_condition(line) {
            *counts.entry(condition.to_owned()).or_default() += 1;
        }
    }

    let prefix = fish_condition_helper_prefix(cmd_name);
    let mut replacements = BTreeMap::<String, String>::new();
    let mut helpers = String::new();
    for (condition, count) in counts {
        if count < 2 {
            continue;
        }

        let helper = format!("{prefix}{}", replacements.len());
        let old_len = format!("-n \"{condition}\"").len();
        let new_len = format!("-n \"{helper}\"").len();
        let helper_len = format!("function {helper}\n    {condition}\nend\n").len();
        if old_len <= new_len || (old_len - new_len) * count <= helper_len {
            continue;
        }

        helpers.push_str(&format!("function {helper}\n    {condition}\nend\n"));
        replacements.insert(condition, helper);
    }

    if replacements.is_empty() {
        return completion;
    }

    let mut out = String::with_capacity(completion.len().saturating_sub(helpers.len()));
    let mut inserted_helpers = false;
    for line in completion.lines() {
        if !inserted_helpers && line.starts_with("complete ") {
            out.push_str(&helpers);
            inserted_helpers = true;
        }

        if let Some(condition) = fish_complete_condition(line)
            && let Some(helper) = replacements.get(condition)
        {
            out.push_str(&line.replacen(
                &format!("-n \"{condition}\""),
                &format!("-n \"{helper}\""),
                1,
            ));
            out.push('\n');
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
}

fn fish_complete_condition(line: &str) -> Option<&str> {
    if !line.starts_with("complete ") {
        return None;
    }

    let (_, rest) = line.split_once(" -n \"")?;
    let (condition, _) = rest.split_once('"')?;
    Some(condition)
}

fn fish_condition_helper_prefix(cmd_name: &str) -> String {
    let short_name: String =
        cmd_name.chars().filter(|c| c.is_ascii_alphanumeric()).take(2).collect();
    format!("__f{short_name}")
}
