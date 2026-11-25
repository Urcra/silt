# A Silt plugin

A template for a plugin of [Silt](https://github.com/Urcra/silt)'s: Notes, which lists and adds notes in a notes service over HTTP. Copy it, rename it, and point it at your own API. The guide's page [Writing a plugin](https://github.com/Urcra/silt/blob/main/docs/guide/writing-plugins.md) explains each part.

- `silt-plugin.toml`: what the plugin is and everything it may do: its capabilities with their reasons, its credential, and its actions with their default rules. Silt holds it to that.
- `src/lib.rs`: its tools, how it prepares each call (the card people see and the exact requests it will send), and how it runs one.
- `tests/plugin.rs`: the plugin in `silt-plugin-test`, a stand-in for Silt's host, which answers its requests and records them.

```sh
rustup target add wasm32-wasip2
cargo test            # builds it for wasm32-wasip2 and runs it in the stand-in host
silt plugin dev .     # installs it as yours; --watch rebuilds it on change
silt plugin enable example.notes --workspace work
```

People add it from its repository with `silt plugin add https://github.com/you/your-plugin#v0.1.0`, which shows them what it gets before it's built.

Before publishing: give it your own `id` (`<publisher>.<name>`), `publisher`, `repository` and `license`, and pin the `toolchain` you build it with.
