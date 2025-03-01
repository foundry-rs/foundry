use console::style;

#[allow(clippy::ptr_arg)]
fn check_username_is_available(x: &String) -> Result<(), &'static str> {
    if ["alice", "bob"].contains(&x.as_str()) {
        Err("Username already taken")
    } else {
        Ok(())
    }
}

fn main() -> std::io::Result<()> {
    // Set a no-op Ctrl-C to make it behave as `Esc` (see the basic example for details).
    ctrlc::set_handler(move || {}).expect("setting Ctrl-C handler");

    cliclack::clear_screen()?;
    cliclack::intro(style(" interactive validation ").on_cyan().black())?;

    let username: String = cliclack::input("Username (not 'alice' or 'bob')")
        .validate_interactively(|x: &String| (x.len() > 2).then_some(()).ok_or("too short"))
        .validate_on_enter(check_username_is_available)
        .interact()?;

    let _password = cliclack::password("Provide a password")
        .mask('▪')
        .validate_interactively(|x: &String| {
            if x.len() < 8 {
                Err("password should be at least 8 characters long")
            } else {
                Ok(())
            }
        })
        .interact()?;

    cliclack::note("User created", format!("{username}\n▪▪▪▪▪\n"))?;

    Ok(())
}
