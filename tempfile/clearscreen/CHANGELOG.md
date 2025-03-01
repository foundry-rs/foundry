
## v4.0.1 (2025-01-05)

- **Deps:** Update MSRV to 1.79 - ([6984714](https://github.com/watchexec/clearscreen/commit/69847147d4deea47808e317d8b5f34b16a616ef2))
- **Deps:** Switch from winapi to windows-sys (#26) - ([a13ea5a](https://github.com/watchexec/clearscreen/commit/a13ea5a6da2163a7f1efa8625f74cf84505c0845))
- **Repo:** Fix changelog format - ([d72ec08](https://github.com/watchexec/clearscreen/commit/d72ec08ad20fd3b32434ed285f062cd5d3e795c0))

## v4.0.0 (2025-01-01)

- **Deps:** Update which to 7.0.0 (#25) - ([207b3a4](https://github.com/watchexec/clearscreen/commit/207b3a4fdf7109faefc699250fb710dcfda18b83))
- **Deps:** Add lockfile to git - ([64180fe](https://github.com/watchexec/clearscreen/commit/64180fe2d7db612633a77337159b020fbac93e68))
- **Deps:** Upgrade thiserror to 2.0.9 - ([2b4b16f](https://github.com/watchexec/clearscreen/commit/2b4b16f6d18fefc2324a36dc4f005c84b5245684))
- **Deps:** Upgrade nix to 0.29.0 - ([556fd47](https://github.com/watchexec/clearscreen/commit/556fd4719a527fac463b6d70ea0539ff72bb0f93))
- **Documentation:** Update wezterm information - ([dd1430a](https://github.com/watchexec/clearscreen/commit/dd1430a5f8d106f4e9d5951e7c1358202c0994d2))
- **Feature:** Support wezterm (#23) - ([4225aae](https://github.com/watchexec/clearscreen/commit/4225aae53a68720072bcaa76edb1e00362684218))
- **Feature:** Decouple public API from dependencies - ([165fe96](https://github.com/watchexec/clearscreen/commit/165fe96b0f6a918d093001b827517c0e65c5dace))
- **Repo:** Replace custom script with cargo-release - ([8835973](https://github.com/watchexec/clearscreen/commit/8835973168a3c422afbcde4f39e1b60b3a87c795))
- **Repo:** Use cliff for changelog - ([0d4fe66](https://github.com/watchexec/clearscreen/commit/0d4fe669f7f4625ea86414d8d18aeab3e3c70fc8))

## v3.0.0 (2024-04-11)

- Update to nix 0.28. ([#21](https://github.com/watchexec/clearscreen/pull/21), thanks [@charliermarsh](https://github.com/charliermarsh))
- Update to which 6. ([#19](https://github.com/watchexec/clearscreen/pull/19))
- Update MSRV to 1.72.

## v2.0.1 (2023-04-04)

## v2.0.0 (2022-12-28)

- Don't use BORS.
- Update dependencies.
- Update to nix 0.26.
- Change MSRV policy to stable-5 supported, and bump MSRV to 1.60.0.
- Handle tmux explicitly ([#9](https://github.com/watchexec/clearscreen/pull/9)).
- Fall back to hardcoded sequence if terminfo is not available ([#9](https://github.com/watchexec/clearscreen/pull/9)).

## v1.0.10 (2022-06-01)

- Use BORS.
- Update to nix 0.24, limit features to only those used ([#6](https://github.com/watchexec/clearscreen/pull/6)).

## v1.0.9 (2021-12-02)

- Change CI test to test Windows 10 detection with a manifested test executable.
- Clarify in documentation the expected behaviour of `is_windows_10()` and what is or not a bug.

## ~~v1.0.8 (2021-12-02)~~ (yanked)

- Stop checking powershell's `PackageManagement` capability as a Win10 check
  ([#5](https://github.com/watchexec/clearscreen/issues/5)).

## v1.0.7 (2021-08-26)

- Flush after E3 sequence in `Terminfo` ([#4](https://github.com/watchexec/clearscreen/issues/4)).

## v1.0.6 (2021-07-22)

- Omit unsupported UTF8 input flag on non-Linux.

## v1.0.5 (2021-07-22)

- Update to nix 0.22.

## v1.0.4 (2021-05-22)

- Fix [#1](https://github.com/watchexec/clearscreen/issues/1): need to flush after writing sequences.

## v1.0.3 (2021-05-08)

- Drop unused `log` dependency.
- Generalise iTerm workaround from 1.0.1 to default behaviour on macOS when the `TERM` starts with
  `xterm` and the terminfo does not have `E3`.
- Hide `WindowsConsoleClear` and `WindowsConsoleBlank` under an undocumented feature as they are
  buggy/do not work as per my testing on Win10. `WindowsVtClear` and `Cls` are sufficient for clear.

## v1.0.2 (2021-04-29)

- Use `VtRis` for Kitty when using its own terminfo.
- Use `VtRis` for SyncTERM, Tess, any rxvt, Zellij, and Zutty.
- Use `XtermClear` for these when using their own terminfo:
  - GNOME Terminal,
  - Konsole,
  - screen,
  - Termite,
  - XFCE4 Terminal.

## v1.0.1 (2021-04-26)

- Use `XtermClear` on iTerm on macOS when `TERM` starts with `xterm`, to work around macOS not
  having the correct terminfo by default.

## v1.0.0 (2021-04-25)

Initial release
