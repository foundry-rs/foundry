Research on Terminals
=====================

All tested with their latest version obtainable of Arch Linux (or macOS 11, Windows 10) as of
writing. Version tested is noted where possible, but otherwise compare to the git blame date.

To contribute entries:

- Insert in the correct category, in lexicographic order
- Test with both the terminal’s own terminfo, and with `xterm-256color`.
- If the terminal doesn’t have its own terminfo, note that, and note which it is trying to emulate.
  - And consider filing a bug to tell them to provide their own terminfo!
- If a terminal has forks, especially if there’s a lot of them, only document a fork if its
  behaviour is different.
- If the terminal is based on a common library, mention it.
- If the terminal is web-based, mention that.
- Document the current selection of `::default()`.
- Document the behaviour of at least:
  - `Terminfo`
  - `TerminfoScreen`
  - `TerminfoScrollback`
  - `VtRis`
  - `XtermClear`
- “Normal” behaviour refers to:
  - `::default()`: screen and scrollback (if at all possible) cleared
  - `Terminfo`: at least screen cleared, and optionally scrollback
  - `TerminfoScreen`: only screen cleared
  - `TerminfoScrollback`: only scrollback cleared
  - `VtRis`: screen and scrollback cleared, and (at least some modes of) terminal reset
  - `XtermClear`: screen and scrollback cleared
  - `Cls`: screen and scrollback cleared
  - `WindowsVtClear`: screen and scrollback cleared
- There is zero tolerance for advertising via this document.

How to test:
------------

First link the clscli example program into your PATH, e.g.

```
ln -s $(pwd)/target/debug/examples/clscli ~/.local/share/bin/clscli
```

Open the terminal in its default profile, or as it comes when first installed.

Then use `env | grep TERM` to see what the `TERM` and other related variables look like (make note!).

Look into `/usr/share/terminfo` for a terminfo that matches the terminal, or wherever it is on your
system. If there's a separate but official package for the terminal’s terminfo, use it.

First test with the native terminfo: set it either in the terminal’s settings, or use
`env TERM=name $SHELL`, then with the `TERM` the terminal first starts with by default, and finally
with `xterm-256color` if that’s not been covered yet.

 1. First run `clscli auto`. Look quick, the name of the variant selected by default will be printed,
    and one second later, hopefully, the screen will clear. Document that variant.
 2. Then run `clscli Variant` where the variant is: `Terminfo`, `TerminfoScreen`,
    `TerminfoScrollback`, `VtRis`, `XtermClear`, and the variant discovered in 1, if not one of
    these. Before each, run `seq 1 100` or something like it to fill the screen and some scrollback.
    Document the behaviour if it differs from normal, or state “normal.”
 3. Optionally (if you want), if `clscli auto` does not exhibit the normal behaviour, open an issue
    and provide enough details to be able to modify the `::default()` selection to select a
    different default that works. If you’re really enthusiastic, you can even open a PR with it!
 4. To submit your research, either submit a PR to this file (preferred, you can even do it in the
    GitHub Web UI), or open an issue with your research (I’ll merge it in), or send me an email.

Platforms
---------

On macOS, the terminfo for `xterm` and variants does not by default include E3, which makes
`Terminfo` not clear scrollback and `TerminfoScrollback` return an error (E3 not found), even when
the terminal in question actually does support E3. For that reason, default behaviour on macOS is
switched to use `XtermClear` if the `TERM` starts with `xterm` and the terminfo doesn’t have E3.

If the terminfo database is not available, `::default()` falls back to `XTermClear` instead of
supplying a useless `Terminfo`. When testing, it's expected to have a functional terminfo where
practical.

Emulator libraries
------------------

### BearLibTerminal

### libamxt

### libt3widget

### libt3window

### libterm

### libtickit

### libtsm

### libvterm

### Qtermwidget

### Rote

### VTE

When “VTE-based” is stated and nothing else, assume this:

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.


Emulators
---------

### Alacritty

- Version 0.7.2

With native `TERM=alacritty`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Aminal

- Version Nightly-develop-2020-01-26-4033a8b

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`. **The better option would be `VtRis`, but there’s no way to tell we’re
  running in Aminal.**
- `Terminfo`: does not clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does not clear scrollback, erases the screen, but leaves cursor position
  intact, i.e. at the bottom of the screen if we were there.
- `VtRis`: clears screen, doesn’t clear scrollback, but does push the existing output up, so that
  information is not lost.
- `XtermClear`: as for `Terminfo`.

### Android Terminal Emulator

### Archipelago

- Web-based

### ate

- Version 1.0.1

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Blink Shell (iOS)

### Bterm

- Version 2.0.0

Native `TERM` is `xterm`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Butterfly

- Web-based

### Cathode

### CMD.EXE

- Windows 10 Pro, build 19042.630

There's no `TERM` variable and no terminfo database.

- Default: `WindowsVtClear`.
- `Terminfo`: there's no TERM nor terminfo database.
- `TerminfoScreen`: there's no TERM nor terminfo database.
- `TerminfoScrollback`: there's no TERM nor terminfo database.
- `VtRis`: prints `←c` and does nothing else.
- `XtermClear`: prints `←[H←[2J←[3J` and does nothing else.
- `Cls`: normal.
- `WindowsConsoleClear`: does nothing ***BUG!***
- `WindowsConsoleBlank`: does nothing ***BUG!***
- `WindowsVtClear`: normal.

### ConEmu

- Version 210422

There's no `TERM` variable and no terminfo database.

- Default: `WindowsVtClear`.
- `Terminfo`: there's no TERM nor terminfo database.
- `TerminfoScreen`: there's no TERM nor terminfo database.
- `TerminfoScrollback`: there's no TERM nor terminfo database.
- `VtRis`: normal.
- `XtermClear`: normal.
- `Cls`: normal.
- `WindowsVtClear`: normal.

### ConsoleZ

- Version 1.19.0.19104

There's no `TERM` variable and no terminfo database.

- Default: `WindowsVtClear`.
- `Terminfo`: there's no TERM nor terminfo database.
- `TerminfoScreen`: there's no TERM nor terminfo database.
- `TerminfoScrollback`: there's no TERM nor terminfo database.
- `VtRis`: prints `←c`, does nothing else.
- `XtermClear`: prints `←[H←[2J←[3J`, does nothing else.
- `Cls`: normal.
- `WindowsVtClear`: normal.

### Cool Retro Term

- Version 1.1.1

Native `TERM` is `xterm`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: scrollback not cleared.
- `XtermClear`: normal.

### Core Terminal

- Version 4.2.0
- Doesn’t respect user shell by default.

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: scrollback not cleared.
- `XtermClear`: normal.

### Deepin Terminal

- Version 5.4.0.6

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: scrollback not cleared.
- `XtermClear`: normal.

#### Old GTK version

- Version 5.0.4.3

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Dinu

### dmenu-term?

### domterm

- Web-based?

### dwt

- Version 0.6.0
- VTE-based

### eDEX UI

- Version 2.2.7
- Doesn’t respect user shell by default.

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Electerm

- Version 1.11.16
- Doesn’t respect user shell by default.

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`. **The better option would be `VtRis`, but there’s no way to tell we’re
  running in Electerm.**
- `Terminfo`: normal, except scrollbar is weird, like it thinks there’s still all the old content,
  but without showing any scrolling when going up or down.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: as for `Terminfo`.

### Elokab Terminal

- Arabic language support!

### eterm

### Evil VTE

- VTE-based
- Untested yet

### ExtraTerm

- Version 0.58.0

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`. (Mostly because it’s the least worst and has a chance to get better.)
- `Terminfo`: does not clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: behaves like `Terminfo` but also prints `[2m` (badly handled unknown escape).
- `XtermClear`: as for `Terminfo`.

### fbpad

### Fingerterm

- For Nokia N9 phones?

### Fluent Terminal (Windows)

- Version 0.7.5.0
- Xterm.js-based

There's no `TERM` variable and no terminfo database.

- Default: `WindowsVtClear`.
- `Terminfo`: there's no TERM nor terminfo database.
- `TerminfoScreen`: there's no TERM nor terminfo database.
- `TerminfoScrollback`: there's no TERM nor terminfo database.
- `VtRis`: does not clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `XtermClear`: as for `VtRis`.
- `Cls`: as for `VtRis`.
- `WindowsVtClear`: as for `VtRis`.

### Foot

- Version 1.7.2
- Wayland only

With `TERM=foot`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### FQTerm

- Version 0.9.10.1.1.g55d08df

With `TERM=vt102`:

- Default: `Terminfo`.
- `Terminfo`: doesn’t clear scrollback.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: doesn’t support E3.
- `VtRis`: does nothing.
- `XtermClear`: doesn’t clear scrollback.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: doesn’t clear scrollback.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does nothing.
- `VtRis`: does nothing.
- `XtermClear`: doesn’t clear scrollback.

### Germinal

- Version 26
- VTE-based

### Guake

- Version 3.7.0
- VTE-based

### GNOME Terminal

- Version 3.40.0
- VTE-based

With `TERM=gnome-256color`:

- Default: `XTermClear`.
- `Terminfo`: behaves like `TerminfoScreen`, doesn’t clear scrollback.
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Goterminal

### Havoc

- Wayland only

### Hyper

- Web-based

### iTerm2

- Version 3.3.12

Native `TERM` is `xterm-256color`.

- Default: `XtermClear`.
- `Terminfo`: normal (does not clear scrollback).
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: does not clear scrollback (behaves like `TerminfoScreen`).
- `XtermClear`: normal.

### jbxvt

### jfbterm

### JuiceSSH

- Version

Native `TERM` is `linux`.

- Default: `Terminfo`.
- `Terminfo`: does not clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: does nothing
- `XtermClear`: as for `Terminfo`.

### Kermit

- Version 3.4
- VTE-based

The `kermit` terminfo also exists, but may not be related, and does not work.

### King’s Cross (kgx)

- Version 0.2.1

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Kitty

- Version 0.20.1

With native `TERM=xterm-kitty`:

- Default: `VtRis`.
- `Terminfo`: does not clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=kitty`: as with `xterm-kitty`.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: erases scrollback and screen, but does not clear them (can be scrolled, but
  all is blank).
- `VtRis`: normal.
- `XtermClear`: normal.

### KMScon

### Konsole

- Version 21.04.0

With native `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `XtermClear`: normal.

With `TERM=konsole`:

- Default: `XtermClear`.
- `Terminfo`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `XtermClear`: normal.

### Lilyterm

- libvte-based

### Liri Terminal

- Version 0.2.0
- Doesn’t respect user shell by default.

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: doesn’t clear scrollback.
- `XtermClear`: normal.

### Literm

- fingerterm-based?

### lwt

- Version 2020-12-02
- VTE-based
- Doesn’t respect user shell by default.

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: normal.
- `VtRis`: doesn’t clear scrollback.
- `XtermClear`: normal.

### LX Terminal

- Version 0.4.0
- VTE-based

### MacTerm

- Version 5 for macOS =>10.15 in development, I don't have an older mac to test 4.x.

### MacWise

- Version 21.6
- In VT100 emulation mode
- Does not have a native `TERM`.

With `TERM=vt100`:

- Default: `Terminfo`.
- `Terminfo`: erases the screen without scrolling up, thus losing info, then inserts a screenful of
  whitespace, then scrolls up. Does not clear scrollback.
- `TerminfoScreen`: as for `Terminfo`.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: does not clear scrollback, does not reset style.
- `XtermClear`: scrolls screen up, then fills the screen with whitespace, places the cursor at the
  bottom right, then prints `3.2$`, then does that once again. (???)

With `TERM=xterm-256color`:

- Default: `XtermClear`. (`Terminfo` would be better, but impossible to detect.)
- `Terminfo`: normal. Does not clear scrollback.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: does not clear scrollback, does not reset style.
- `XtermClear`: as with `TERM=vt100`.

### Mantid

- Version 1.0.6
- VTE-based

### MATE Terminal

- Version 1.24.1
- VTE-based

### Maui Station

- Version 1.2.1

Native `TERM` is `xterm`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: doesn’t clear scrollback.
- `XtermClear`: normal.

### Microsoft Terminal / Windows Terminal

- Version 1.7.1033.0

There's no `TERM` variable and no terminfo database.

- Default: `XtermClear`.
- `Terminfo`: there's no TERM nor terminfo database.
- `TerminfoScreen`: there's no TERM nor terminfo database.
- `TerminfoScrollback`: there's no TERM nor terminfo database.
- `VtRis`: normal.
- `XtermClear`: normal.
- `WindowsVtClear`: normal.
- `Cls`: normal.

### Miniterm

- Version 1.7.0
- VTE-based

### MinTTY (Windows)

- Version 3.1.6
- PuTTY-based?
- Via Git-Bash

Native `TERM` is `xterm`

- Default: `WindowsVtClear`.
- `Terminfo`: there's no terminfo database.
- `TerminfoScreen`: there's no terminfo database.
- `TerminfoScrollback`: there's no terminfo database.
- `VtRis`: normal.
- `XtermClear`: normal.
- `Cls`: does nothing.
- `WindowsVtClear`: normal.

### Miro

- Version 0.2.0

### MLTERM

- Version 3.9.0

Native `TERM` is `xterm`.

- Default: `Terminfo`. (No real good option here.)
- `Terminfo`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `XtermClear`: as for `Terminfo`.

### MobaXterm

- Version 21.1 build 4628 Home Edition

There's no `TERM` variable and no terminfo database.

- Default: `WindowsVtClear`.
- `Terminfo`: there's no TERM nor terminfo database.
- `TerminfoScreen`: there's no TERM nor terminfo database.
- `TerminfoScrollback`: there's no TERM nor terminfo database.
- `VtRis`: prints `←c`, does nothing else.
- `XtermClear`: prints `←[H←[2J←[3J`, does nothing else.
- `Cls`: doesn’t clear scrollback.
- `WindowsVtClear`: doesn’t clear scrollback.

#### With built-in Bash mode

Native `TERM` is `xterm`.

- Default: `WindowsVtClear`.
- `Terminfo`: there's no terminfo database.
- `TerminfoScreen`: there's no terminfo database.
- `TerminfoScrollback`: there's no terminfo database.
- `VtRis`: doesn’t clear scrollback.
- `XtermClear`: normal.
- `Cls`: doesn’t clear scrollback.
- `WindowsVtClear`: normal.

### mrxvt

### mt

### Nautilus Terminal

- Version 3.5.0
- VTE-based

### Nemo Terminal

- Version 4.8.0
- VTE-based

### Neovim

- Version 0.4.4

`TERM` is inherited. With `xterm-256color`:

- Default: `Terminfo`. (No real good option here.)
- `Terminfo`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `XtermClear`: as for `Terminfo`.

### Orbterm

### Pangoterm

- libvterm-based

### Pantheon/Elementary Terminal

- Version 5.5.2

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### PowerCmd

### PuTTY

- Version 0.74

With native `TERM=xterm`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: does not clear scrollback.
- `XtermClear`: normal.

With `TERM=putty`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does nothing.
- `VtRis`: does not clear scrollback.
- `XtermClear`: normal.

### QML Konsole

- Version 0.1.r2.g81e74ad

Native `TERM` is `xterm`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does nothing.
- `VtRis`: does not clear scrollback.
- `XtermClear`: normal.

### Qt DOM term

### Qterminal

- Version 0.16.1

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### rcfvt

- Version r66.d390d61

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### ROXTerm

- Version 3.10.1
- VTE-based

### Runes

### Sakura

- Version 3.8.1
- VTE-based

### sdvt

### Snowflake

### st

- Version 0.8.4

With `TERM=st-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: also clears scrollback.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: also clears scrollback.
- `TerminfoScrollback`: does nothing.
- `VtRis`: normal.
- `XtermClear`: normal.

### sterm

- Version 0.1.2
- VTE-based

Native `TERM` is `xterm-256color`.

There’s no scrollback at all, so it’s impossible to know how things are really handled, but 🤷.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### stgl

### StupidTerm

- Version 1.r24.gf824e41
- VTE-based

### Syncterm

- Version 1.1

Native `TERM` is `syncterm`.

- Default: `VtRis`.
- `Terminfo`: no terminfo found.
- `TerminfoScreen`: no terminfo found.
- `TerminfoScrollback`: no terminfo found.
- `VtRis`: normal.
- `XtermClear`: does not clear scrollback.

### Taterm

- Version 12

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Terminal.app (GNUstep)

### Terminal.app (macOS)

- Version 2.10 (433)

Native `TERM` is `xterm-256color`.

- Default: `XtermClear`.
- `Terminfo`: normal (does not clear scrollback).
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: erases the screen without scrolling up (not abnormal) and does not clear scrollback.
- `XtermClear`: normal.

### Terminaleco

### Terminalpp

### Terminate

- Version 0.5
- VTE-based
- _Requires_ a TERM to be set, doesn’t manage to get set up properly without.
- There’s no scrollback at all, so it’s impossible to know how things are really handled, but 🤷.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Terminator

- Version 2.1.1

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Terminol

### Terminology

- Version 1.9.0

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Terminus

### Termistor

- Wayland only

### Termit

- Version 3.1.r4.g29bbd1b
- VTE-based

### Termite

- Version 15
- VTE-based

With native `TERM=xterm-termite`:

- Default: `XTermClear`.
- `Terminfo`: normal (doesn’t clear scrollback).
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=termite`:

- Default: `XTermClear`.
- `Terminfo`: normal (doesn’t clear scrollback).
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Termius

- Version 7.9.0

Native `TERM` is `xterm`.

- Default: `Terminfo`. **The better option would be `VtRis`, but there’s no way to tell we’re
  running in Termius.**
- `Terminfo`: normal, except scrollbar is weird, like it thinks there’s still all the old content,
  but without showing any scrolling when going up or down.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal, except scrollbar is even weirder, like it thinks there’s still all the old
  content, but without _allowing the screen to be scrolled at all._ Once the screen fills up again,
  the scrollbar resets.
- `XtermClear`: as for `Terminfo`.

### Termy

- Version 0.3.0
- By nature, the prompt remains at the top, and every command clears the screen.
- However, running a shell inside the terminal makes it behave as usually expected, so that's how
  this is tested.

Native `TERM` is `xterm-256color`.

- Default: `XtermClear`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. (Doesn’t clear scrollback.)
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

### Terra

### Tess

- Version 1.2r65.12944dd
- Doesn’t respect user shell by default.

Native `TERM` is `xterm-color`.

- Default: `VtRis`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. (Doesn’t clear scrollback.)
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal, except scrollbar is weird, like it thinks there’s still all the old content,
  but without showing any scrolling when going up or down.

### The Terminal

### TreeTerm

### Tilda

- Version 1.5.4

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Tilix

- Version 1.9.4

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Tinyterm

- VTE-based
- Untested yet

### Topinambour

- VTE-based
- Untested yet

### Tortosa

- VTE-based
- Untested yet

### Ume

- Version r67.242a9f5
- VTE-based

### urxvt

- Version 9.22

With native `TERM=rxvt-unicode-265color`:

- Default: `VtRis`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. (Doesn’t clear scrollback.)
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: normal.
- `XtermClear`: as for `Terminfo`.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. (Doesn’t clear scrollback.)
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: normal.
- `XtermClear`: as for `Terminfo`.

### uterm

- libtsm-based

### uuterm

- Version 80
- There’s no scrollback at all, so it’s impossible to know how things are really handled, but 🤷.

With native `TERM=uuterm`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`: very broken, but clearing works as normal.

### Viter

- Version r166.c8ca21a

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### vt100-parser

### Wayst

- Version r223.e72ca78

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`. **The better option would be `VtRis`, but there’s no way to tell we’re
  running in Wayst.**
- `Terminfo`: normal, doesn’t clear scrollback.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: clears the screen, keeping the cursor position the same, but doesn’t clear
  scrollback!
- `VtRis`: normal.
- `XtermClear`: doesn’t clear scrollback.

### Wezterm

- Version 20240203

Native `TERM` is `wezterm`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### WindTerm

- Version 2.1.0 (Win10 version)

There's no `TERM` variable and no terminfo database.

- Default: `WindowsVtClear`.
- `Terminfo`: there's no TERM nor terminfo database.
- `TerminfoScreen`: there's no TERM nor terminfo database.
- `TerminfoScrollback`: there's no TERM nor terminfo database.
- `VtRis`: does not clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `XtermClear`: as for `VtRis`.
- `Cls`: as for `VtRis`.
- `WindowsVtClear`: as for `VtRis`.

### Wlterm

- libtsm-based

### wlgxterm

### XFCE4 Terminal

- Version 0.8.10
- VTE-based

With `TERM=xfce`:

- Default: `XTermClear`.
- `Terminfo`: behaves like `TerminfoScreen`, doesn’t clear scrollback.
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### xiate

- Version 20.07

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Xterm

- Version 367

Native `TERM` is `xterm`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: normal, and dings the terminal bell.
- `XtermClear`: normal.

### Yaft

### Yaftx

- Version 0.2.9
- There’s no scrollback at all, so it’s impossible to know how things are really handled, but 🤷.

With native `TERM=yaft-265color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: normal.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does nothing.
- `VtRis`: normal.
- `XtermClear`: normal.

### Yakuake

- Version 21.04.0
- Konsole-based

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: normal.
- `VtRis`: doesn’t clear scrollback, appears to clear the screen, but really erases the screen
  without scrolling the existing output up, thus losing a screenful of information.
- `XtermClear`: normal.

### z/Scope

- Web-based?

### ZOC

- Version 8 (8023)

Native `TERM` is `xterm-256color`.

- Default: `XtermClear`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: does not clear scrollback.
- `XtermClear`: normal.

### Zterm

### Zutty

- Version 0.8

Native `TERM` is `xterm-256color`.

- Default: `VtRis`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. Doesn’t clear scrollback.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: normal.
- `XtermClear`: as for `Terminfo`.


Serial terminal emulators?
-------------------------

### Bootterm

### Coolterm

### Cutecom

### dterm

### Easyterm

### HTerm

### iserterm

### Microcom

### Minicom

### Moserial

### Picocom

### ssterm

### tio


Multiplexers
------------

### 3mux

- Version 1.1.0

Native `TERM` is `xterm-256color`.

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: **erases the screen like `TerminfoScreen`** and clears scrollback.
- `VtRis`: does nothing.
- `XtermClear`: normal.

### Byobu

- Uses Tmux underneath

### Dvtm

- Version 0.15

With native `TERM=dvtm-265color`:

- Default: `Terminfo`. (The least worse option.)
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. Doesn’t clear scrollback.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: does nothing.
- `XtermClear`: as for `Terminfo`.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. Doesn’t clear scrollback.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: does nothing.
- `XtermClear`: as for `Terminfo`.

### Eternal Terminal

### Mosh

- Version 1.3.2
- `TERM` is inherited.
- There’s no scrollback at all, so it’s impossible to know how things are really handled, but 🤷.

Tested here with `xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: does nothing.
- `VtRis`: normal.
- `XtermClear`: normal.

### mtm

- Version r394.b14e99c

With native `TERM=screen-265color-bce`:

- Default: `XtermClear`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. Doesn’t clear scrollback.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: as for `Terminfo`.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: clears scrollback **and screen**, but leaves the cursor position.
- `VtRis`: as for `TerminfoScreen`.
- `XtermClear`: normal.

### Screen

- Version 4.08.00

With `TERM=screen`:

- Default: `XtermClear`.
- `Terminfo`: normal (does not clear scrollback).
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: terminfo does not support E3.
- `VtRis`: adds a screenful of space to the scrollback before clearing, does not clear scrollback.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: **clears scrollback**, even though `TerminfoScrollback` below doesn’t work.
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: doesn’t do anything.
- `VtRis`: adds a screenful of space to the scrollback before clearing, does not clear scrollback.
- `XtermClear`: normal.

### Tab-rs

- Version 0.5.7
- Scrollback is inherited from the terminal, not managed internally, so depends on what you have.
- `TERM` is inherited too, so as long as it passes the escapes out, it will work as the terminal.

Tested with `xterm-256color` in Alacritty:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: normal.
- `XtermClear`: normal.

### Tmux

- Version 3.2

With `TERM=tmux-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: normal.
- `TerminfoScrollback`: normal.
- `VtRis`: does not clear scrollback.
- `XtermClear`: normal.

With `TERM=xterm-256color`:

- Default: `Terminfo`.
- `Terminfo`: normal.
- `TerminfoScreen`: adds a screenful of space to the scrollback before clearing.
- `TerminfoScrollback`: normal.
- `VtRis`: does not clear scrollback.
- `XtermClear`: normal.

### Zellij

- Version 0.5.1
- `TERM` is inherited.

Tested with `xterm-256color` in Alacritty:

- Default: `VtRis`.
- `Terminfo`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information. Doesn’t clear scrollback.
- `TerminfoScreen`: appears to clear the screen, but really erases the screen without scrolling the
  existing output up, thus losing a screenful of information.
- `TerminfoScrollback`: does nothing.
- `VtRis`: normal.
- `XtermClear`: as for `Terminfo`.


Recorders
---------

### Asciinema

### Asciinema Rust

### GoTTY

### Hasciinema?

### ipbt

### Shell in a box

### Shellshare

### Showterm

### T-Rec

### Term to SVG

### Terminalizer

### Termrec

### tmate.io

### ts-player

### tty-share

### TTYcast

### ttyd

### upterm

### webtty
