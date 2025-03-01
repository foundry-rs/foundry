# Changelog

## Next (YYYY-MM-DD)

## v3.0.3 (2025-02-09)

## v3.0.2 (2024-10-14)

- Deps: gix-config 0.40

## v3.0.1 (2024-04-28)

- Hide fmt::Debug spew from ignore crate, use `full_debug` feature to restore.

## v3.0.0 (2024-04-20)

- Deps: gix-config 0.36
- Deps: miette 7

## v2.1.0 (2024-01-04)

- Normalise paths on all platforms (via `normalize-path`).
- Require paths be normalised before discovery.
- Add convenience APIs to `IgnoreFilesFromOriginArgs` for that purpose.

## v2.0.0 (2024-01-01)

- A round of optimisation by @t3hmrman, improving directory traversal to avoid crawling unneeded paths. ([#663](https://github.com/watchexec/watchexec/pull/663))
- Respect `applies_in` scope when processing nested ignores, by @thislooksfun. ([#746](https://github.com/watchexec/watchexec/pull/746))

## v1.3.2 (2023-11-26)

- Remove error diagnostic codes.
- Deps: upgrade to gix-config 0.31.0
- Deps: upgrade Tokio requirement to 1.33.0

## v1.3.1 (2023-06-03)

- Use Tokio's canonicalize instead of dunce::simplified.

## v1.3.0 (2023-05-14)

- Use IO-free dunce::simplify to normalise paths on Windows.
- Handle gitignores correctly (one GitIgnoreBuilder per path).
- Deps: update gix-config to 0.22.

## v1.2.0 (2023-03-18)

- Deps: update git-config to gix-config.
- Deps: update tokio to 1.24
- Ditch MSRV policy (only latest supported now).
- `from_environment()` no longer looks at `WATCHEXEC_IGNORE_FILES`.

## v1.1.0 (2023-01-08)

- Add missing `Send` bound to async functions.

## v1.0.1 (2022-09-07)

- Deps: update git-config to 0.7.1
- Deps: update miette to 5.3.0

## v1.0.0 (2022-06-16)

- Initial release as a separate crate.
