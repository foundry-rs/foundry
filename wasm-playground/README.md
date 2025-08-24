# WASM Playground

- Have [Deno](https://deno.land) or any other JavaScript runtime installed
- Add `wasm32-unknown-unknown` target to Rust:
  `rustup target add wasm32-unknown-unknown`
- Then, from project root:

  ```sh
  RUSTFLAGS='--cfg=getrandom_backend="wasm_js"'

  deno run --allow-all jsr:@deno/wasmbuild --project cast-wasm --all-features --out wasm-playground/dist
  ```

- Now check `wasm-playground/main.ts`, then run it with:

  ```sh
  deno run --allow-all wasm-playground/main.ts
  ```

  it imports the WASM module and runs it in the Deno runtime.

- Finally, see it in the browser:

  ```sh
  deno vite dev
  ```
