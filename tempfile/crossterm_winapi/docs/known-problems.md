# Known problems

There are some problems I discovered during development. I don't think it has to do anything with
the crossterm, but it has to do with how terminals handle ANSI or WinAPI. 

## WinAPI

- PowerShell does not interpret 'DarkYellow' and uses gray instead, cmd is working perfectly fine.
- PowerShell inserts an '\n' (enter) when the program starts, this enter is the one you pressed when running the command.
- After the program ran, PowerShell will reset the background and foreground colors.
