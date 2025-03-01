use console::style;

fn main() -> std::io::Result<()> {
    // Set a no-op Ctrl-C to make it behave as `Esc` (see the basic example).
    ctrlc::set_handler(move || {}).expect("setting Ctrl-C handler");

    cliclack::clear_screen()?;

    cliclack::intro(style(" create-app ").on_cyan().black())?;

    // This is the only difference between this snippet and examples/basic.rs
    // You can supply your items dynamically, i.e. from a database or API.
    let items_for_select = vec![
        ("ts", "TypeScript", ""),
        ("js", "JavaScript", ""),
        ("coffee", "CoffeeScript", "oh no"),
    ];

    let _selected_dynamic_item = cliclack::select("Pick a project type")
        .initial_value("ts")
        .items(&items_for_select)
        .interact()?;

    let items_for_multiselect = &[
        ("prettier", "Prettier", "recommended"),
        ("eslint", "ESLint", "recommended"),
        ("stylelint", "Stylelint", ""),
        ("gh-action", "GitHub Action", ""),
    ];

    let _tools = cliclack::multiselect("Select additional tools")
        .initial_values(vec!["prettier", "eslint"])
        .items(items_for_multiselect)
        .interact()?;

    cliclack::outro(format!(
        "Problems? {}\n",
        style("https://example.com/issues").cyan().underlined()
    ))?;

    Ok(())
}
