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
            Self::ClapCompleteShell(ClapCompleteShell::Bash) => {
                generate_compacted(ClapCompleteShell::Bash, cmd, compact_bash_completion, buf)
            }
            Self::ClapCompleteShell(ClapCompleteShell::Fish) => {
                generate_compacted(ClapCompleteShell::Fish, cmd, compact_fish_completion, buf)
            }
            Self::ClapCompleteShell(ClapCompleteShell::Zsh) => {
                generate_compacted(ClapCompleteShell::Zsh, cmd, compact_zsh_completion, buf)
            }
            Self::ClapCompleteShell(shell) => shell.generate(cmd, buf),
            Self::Nushell => Nushell.generate(cmd, buf),
        }
    }
}

fn generate_compacted(
    generator: impl Generator,
    cmd: &clap::Command,
    compact: fn(&str, Vec<u8>) -> String,
    buf: &mut dyn std::io::Write,
) {
    let mut completion = Vec::new();
    generator.generate(cmd, &mut completion);
    let completion = compact(cmd.get_name(), completion);
    buf.write_all(completion.as_bytes()).expect("failed to write completion file");
}

fn compact_bash_completion(cmd_name: &str, completion: Vec<u8>) -> String {
    let completion =
        String::from_utf8(completion).expect("bash completion scripts should be UTF-8");

    let prev_case_start = "            case \"${prev}\" in";
    let prev_case_end = "            esac";
    let mut counts = BTreeMap::<String, usize>::new();
    let lines: Vec<_> = completion.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i] == prev_case_start
            && let Some(end) = lines[i + 1..].iter().position(|line| *line == prev_case_end)
        {
            let end = i + 1 + end;
            let block = format!("{}\n", lines[i..=end].join("\n"));
            *counts.entry(block).or_default() += 1;
            i = end + 1;
            continue;
        }
        i += 1;
    }

    let prefix = bash_helper_prefix(cmd_name);
    let mut replacements = BTreeMap::<String, String>::new();
    let mut helpers = String::new();
    for (block, count) in counts {
        if count < 2 {
            continue;
        }

        let helper = format!("{prefix}{}", replacements.len());
        let replacement = format!("            {helper} && return 0\n");
        let definition = format!("{helper}() {{\n{block}            return 1\n}}\n");
        let old_len = block.len() * count;
        let new_len = replacement.len() * count + definition.len();
        if old_len <= new_len {
            continue;
        }

        helpers.push_str(&definition);
        replacements.insert(block, replacement);
    }

    if replacements.is_empty() {
        return completion;
    }

    let mut out = String::with_capacity(completion.len().saturating_sub(helpers.len()));
    out.push_str(&helpers);
    let mut i = 0;
    while i < lines.len() {
        if lines[i] == prev_case_start
            && let Some(end) = lines[i + 1..].iter().position(|line| *line == prev_case_end)
        {
            let end = i + 1 + end;
            let block = format!("{}\n", lines[i..=end].join("\n"));
            if let Some(replacement) = replacements.get(&block) {
                out.push_str(replacement);
                i = end + 1;
                continue;
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }

    out
}

fn compact_zsh_completion(cmd_name: &str, completion: Vec<u8>) -> String {
    let completion = String::from_utf8(completion).expect("zsh completion scripts should be UTF-8");

    let start = "_arguments \"${_arguments_options[@]}\" : \\";
    let end = "&& ret=0";
    let lines: Vec<_> = completion.lines().collect();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i] == start
            && let Some(block_end) = lines[i + 1..].iter().position(|line| *line == end)
        {
            let block_end = i + 1 + block_end;
            let block = format!("{}\n", lines[i..=block_end].join("\n"));
            *counts.entry(block).or_default() += 1;
            i = block_end + 1;
            continue;
        }
        i += 1;
    }

    let prefix = zsh_helper_prefix(cmd_name);
    let mut replacements = BTreeMap::<String, String>::new();
    let mut helpers = String::new();
    for (block, count) in counts {
        if count < 2 {
            continue;
        }

        let helper = format!("{prefix}{}", replacements.len());
        let replacement = format!("{helper} && ret=0\n");
        let mut body = block.replace("&& ret=0\n", "");
        if body.as_bytes().ends_with(b" \\\n") {
            body.truncate(body.len() - 3);
            body.push('\n');
        }
        let definition = format!("{helper}() {{\n{body}}}\n");
        let old_len = block.len() * count;
        let new_len = replacement.len() * count + definition.len();
        if old_len <= new_len {
            continue;
        }

        helpers.push_str(&definition);
        replacements.insert(block, replacement);
    }

    if replacements.is_empty() {
        return completion;
    }

    let mut out = String::with_capacity(completion.len().saturating_sub(helpers.len()));
    let mut inserted_helpers = false;
    let mut i = 0;
    while i < lines.len() {
        if !inserted_helpers && lines[i].starts_with(&format!("_{cmd_name}_commands()")) {
            out.push_str(&helpers);
            inserted_helpers = true;
        }

        if lines[i] == start
            && let Some(block_end) = lines[i + 1..].iter().position(|line| *line == end)
        {
            let block_end = i + 1 + block_end;
            let block = format!("{}\n", lines[i..=block_end].join("\n"));
            if let Some(replacement) = replacements.get(&block) {
                out.push_str(replacement);
                i = block_end + 1;
                continue;
            }
        }

        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }

    out
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

fn bash_helper_prefix(cmd_name: &str) -> String {
    let short_name: String =
        cmd_name.chars().filter(|c| c.is_ascii_alphanumeric()).take(2).collect();
    format!("__b{short_name}")
}

fn zsh_helper_prefix(cmd_name: &str) -> String {
    let short_name: String =
        cmd_name.chars().filter(|c| c.is_ascii_alphanumeric()).take(2).collect();
    format!("__z{short_name}")
}

fn fish_condition_helper_prefix(cmd_name: &str) -> String {
    let short_name: String =
        cmd_name.chars().filter(|c| c.is_ascii_alphanumeric()).take(2).collect();
    format!("__f{short_name}")
}
