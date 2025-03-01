# Version 0.9.1
- Add `scroll_right` and `scroll_left` functions.
- Add `font_size` to fetch font size.

# Version 0.9.0
- Fix panic on certain event flags. 

# Version 0.8.0
- Changed some return types.
- Improved some internal error handling. 

# Version 0.7.0
- Make resize event return correct screen dimensions instead of buffers size.

# Version 0.6.1
- Make semaphore `Send` and `Sync` again.
- Make `Inner` `Send` and `Sync` again.

# Version 0.6.0
- Added Common traits (`Debug`, `Clone`, etc) to many public facing types,
especially data struct.
- Significantly updated the `input` structs, so that winapi native types are no longer exposed to the library by crossterm_winapi structs.
- Removed PartialOrd from types where it didn't really make sense
- Reimplemented `Console::read_single_input_event` and `Console::read_console_input` to be more efficient, safe, and correct
- Make `Console::read_console_input` not return a `u32`; the numbr of events is the length of the returned vector.


# Version 0.5.1
- Make `Semaphore` implement `Clone`.

# Version 0.5.0
- Add `Semaphore` object handling
- Make `ButtonState` more flexible.

# Version 0.4.0
- The `Handle` API has been reworked to make it `Send` + `Sync` and close the underlying `HANDLE` when dropped.

# Version 0.3.0

- Make read sync block for windows systems ([PR #2](https://github.com/crossterm-rs/crossterm-winapi/pull/2))

# Version 0.2.1

- Maintenance release only
- Moved to a [separate repository](https://github.com/crossterm-rs/crossterm-winapi)

# Version 0.2.0

- `Console::get_handle` to `Console::handle`
