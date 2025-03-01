# 0.4.6 (December 8, 2023)

- Add `Collect` combinator (backported from http-body-util).

# 0.4.5 (May 20, 2022)

- Add `String` impl for `Body`.
- Add `Limited` body implementation.

# 0.4.4 (October 22, 2021)

- Add `UnsyncBoxBody` and `Body::boxed_unsync`.

# 0.4.3 (August 8, 2021)

- Implement `Default` for `BoxBody`.

# 0.4.2 (May 8, 2021)

- Correctly override `Body::size_hint` and `Body::is_end_stream` for `Empty`.
- Add `Full` which is a body that consists of a single chunk.

# 0.4.1 (March 18, 2021)

- Add combinators to `Body`:
  - `map_data`: Change the `Data` chunks produced by the body.
  - `map_err`: Change the `Error`s produced by the body.
  - `boxed`: Convert the `Body` into a boxed trait object.
- Add `Empty`.

# 0.4.0 (December 23, 2020)

- Update `bytes` to v1.0.

# 0.3.1 (December 13, 2019)

- Implement `Body` for `http::Request<impl Body>` and `http::Response<impl Body>`.

# 0.3.0 (December 4, 2019)

- Rename `next` combinator to `data`.

# 0.2.0 (December 3, 2019)

- Update `http` to v0.2.
- Update `bytes` to v0.5.

# 0.2.0-alpha.3 (October 1, 2019)

- Fix `Body` to be object-safe.

# 0.2.0-alpha.2 (October 1, 2019)

- Add `next` and `trailers` combinator methods.

# 0.2.0-alpha.1 (August 20, 2019)

- Update to use `Pin` in `poll_data` and `poll_trailers`.

# 0.1.0 (May 7, 2019)

- Initial release
