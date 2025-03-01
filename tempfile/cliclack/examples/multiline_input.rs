use cliclack::{input, note};

fn main() -> std::io::Result<()> {
    let res: String = input("Normal test")
        .placeholder("Type here...")
        .multiline()
        .interact()?;
    note("Your input is:", res)?;

    let res: usize = input("Only number:")
        .placeholder("Type here...")
        .multiline()
        .interact()?;
    note("Your input is:", res)?;

    let res: String = input("Interactively validation:")
        .multiline()
        .validate_interactively(|s: &String| match s.len() & 1 == 0 {
            true => Ok(()),
            false => Err("The length of the input should be even"),
        })
        .interact()?;
    note("Your input is:", res)?;

    let res: String = input("Default value test:")
        .multiline()
        .default_input("Default value")
        .interact()?;
    note("Your input is:", res)?;

    let res: String = input("Default value with interactively validation test:")
        .multiline()
        .default_input("Default value.")
        .validate_interactively(|s: &String| match s.len() & 1 == 0 {
            true => Ok(()),
            false => Err("The length of the input should be even"),
        })
        .interact()?;
    note("Your input is:", res)?;

    // One line.

    let res: String = input("Normal test (one-line)")
        .placeholder("Type here...")
        .interact()?;
    note("Your input is:", res)?;

    let res: usize = input("Only number (one-line)")
        .placeholder("Type here...")
        .interact()?;
    note("Your input is:", res)?;

    let res: String = input("Interactively validation (one-line)")
        .validate_interactively(|s: &String| match s.len() & 1 == 0 {
            true => Ok(()),
            false => Err("The length of the input should be even"),
        })
        .interact()?;
    note("Your input is:", res)?;

    let res: String = input("Default value test (one-line)")
        .default_input("Default value")
        .interact()?;
    note("Your input is:", res)?;

    let res: String = input("Default value with interactively validation test (one-line)")
        .default_input("Default value.")
        .validate_interactively(|s: &String| match s.len() & 1 == 0 {
            true => Ok(()),
            false => Err("The length of the input should be even"),
        })
        .interact()?;
    note("Your input is:", res)?;
    Ok(())
}
