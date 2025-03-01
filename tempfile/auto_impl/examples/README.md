# Examples

- `error_messages`: contains some incorrect code that showcases the error messages emitted by `auto_impl`
- **`greet_closure`**: simple example showing how to auto impl for `Fn` traits
- **`keep_default_for`**: shows how to use the `#[auto_impl(keep_default_for(...))]` attribute
- `names`: showcases how `auto_impl` chooses new ident names
- **`refs`**: shows how to auto impl for `&` and `Box`


**Note**: if you want to see what the generated impl blocks look like, use the execellent [`cargo expand`](https://github.com/dtolnay/cargo-expand):

```
$ cargo expand --example refs
```
