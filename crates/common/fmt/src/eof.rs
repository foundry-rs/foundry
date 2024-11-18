use comfy_table::{ContentArrangement, Table};
use revm_primitives::{
    eof::{EofBody, EofHeader},
    Eof,
};
use std::fmt::{self, Write};

pub fn pretty_eof(eof: &Eof) -> Result<String, fmt::Error> {
    let Eof {
        header:
            EofHeader {
                types_size,
                code_sizes,
                container_sizes,
                data_size,
                sum_code_sizes: _,
                sum_container_sizes: _,
            },
        body:
            EofBody { types_section, code_section, container_section, data_section, is_data_filled: _ },
        raw: _,
    } = eof;

    let mut result = String::new();

    let mut table = Table::new();
    table.add_row(vec!["type_size", &types_size.to_string()]);
    table.add_row(vec!["num_code_sections", &code_sizes.len().to_string()]);
    if !code_sizes.is_empty() {
        table.add_row(vec!["code_sizes", &format!("{code_sizes:?}")]);
    }
    table.add_row(vec!["num_container_sections", &container_sizes.len().to_string()]);
    if !container_sizes.is_empty() {
        table.add_row(vec!["container_sizes", &format!("{container_sizes:?}")]);
    }
    table.add_row(vec!["data_size", &data_size.to_string()]);

    write!(result, "Header:\n{table}")?;

    if !code_section.is_empty() {
        let mut table = Table::new();
        table.set_content_arrangement(ContentArrangement::Dynamic);
        table.set_header(vec!["", "Inputs", "Outputs", "Max stack height", "Code"]);
        for (idx, (code, type_section)) in code_section.iter().zip(types_section).enumerate() {
            table.add_row(vec![
                &idx.to_string(),
                &type_section.inputs.to_string(),
                &type_section.outputs.to_string(),
                &type_section.max_stack_size.to_string(),
                &code.to_string(),
            ]);
        }

        write!(result, "\n\nCode sections:\n{table}")?;
    }

    if !container_section.is_empty() {
        let mut table = Table::new();
        table.set_content_arrangement(ContentArrangement::Dynamic);
        for (idx, container) in container_section.iter().enumerate() {
            table.add_row(vec![&idx.to_string(), &container.to_string()]);
        }

        write!(result, "\n\nContainer sections:\n{table}")?;
    }

    if !data_section.is_empty() {
        let mut table = Table::new();
        table.set_content_arrangement(ContentArrangement::Dynamic);
        table.add_row(vec![&data_section.to_string()]);
        write!(result, "\n\nData section:\n{table}")?;
    }

    Ok(result)
}
