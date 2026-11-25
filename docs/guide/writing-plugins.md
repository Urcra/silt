# Writing a plugin

A plugin is a git repository with a manifest, `silt-plugin.toml`, and a WebAssembly component built from it for `wasm32-wasip2`. Silt's own plugins are written exactly this way, in `plugins/` of Silt's repository, and are good examples: Folders is the smallest, GitHub the largest. [Plugins](plugins.md) says how people use them.

## Start from the template
Copy `templates/plugin/` from Silt's repository. It's a plugin, Notes, that lists and adds notes through an HTTP API, with its tests and CI. The template and the crates it builds against (`silt-plugin-api` and its WIT world, `silt-plugin`, `silt-access`, `silt-plugin-test` and `silt-plugin-host`) are MIT, unlike the rest of Silt, so your plugin can be under any licence you choose ([ADR 0122](../adr/0122-the-plugin-sdk-is-mit.md)):

```sh
cp -r silt/templates/plugin my-plugin && cd my-plugin
rustup target add wasm32-wasip2
cargo test
silt plugin dev .
```

`silt plugin dev <folder>` builds it and installs it as yours; with `--watch` it builds again whenever the folder changes, and the new build is used from the next call. Turn it on in a workspace with `silt plugin enable <id> --workspace <workspace>`.

## The manifest
`silt-plugin.toml` declares everything the plugin can do, and Silt holds it to that:
- **`id`, `name`, `version`, `publisher`, `description`, `license`:** its id is `<publisher>.<name>`, such as `acme.deploys`.
- **`world`:** `silt:plugin@1`, the interface it's built for.
- **`build`:** `{ cargo = { package = "...", toolchain = "1.99.1" } }`, the toolchain pinned, so it builds the same way on every machine that adds it.
- **`[[capabilities]]`:** each with its `kind` and a `reason` people read before they add it:
  - `http`, to the `hosts` it names, or those a setting names (`hosts_from`);
  - `git`, for mirrors of the repositories it works with, and `agent-files`, to show them to the agent read-only;
  - `thread-read`, to read the agent's checkout;
  - `reach`, to give the agent's sandbox more, as Folders does;
  - `sockets`, `ssh` and `mcp-process`: TCP to the hosts and ports it names, one command over SSH, and a local MCP server it starts. Silt can't check what these do, so each call that uses one is labelled so, and an org can refuse them.
- **`[[credentials]]`:** what each is for, the hosts it goes to, how it's sent (`send`), and how to get one (`how`). Silt adds it to the plugin's requests to those hosts; the plugin never sees it. One can be `same_as` another, so a token serves an API and git.
- **`[[actions]]`:** each a `read` or a `change`, its `default` rule (`allow`, `ask` or `never`), and the `fields` people's rules can match, such as `branch`. `once = true` makes each call approved on its own.
- **`[[settings]]`:** each with its `kind`, `description` and `scope` (the plugin, an environment, a workspace); `secret` ones are kept with `silt plugin secret` and given to the host's `http` only.
- **`environments`:** what picks one, such as `{ label = "repos", field = "repo" }`: a call goes to the environment whose patterns match its field.

## The interface
The world is in `crates/silt-plugin-api/wit/world.wit`. A plugin exports three functions; the SDK, `silt-plugin-api`, gives them to Rust as the `Plugin` trait:
- **`tools()`:** the tools agents see, each for one of its actions.
- **`prepare(tool, input, context)`:** what the call would do: its action, the fields rules match, a `Card` for the person, and its `Effect`s, each request exactly as it will be sent (an HTTP request's body by its sha256, a push by its commit and the commit it moves from). Nothing may change while preparing; reading is fine.
- **`run(tool, input, call, context)`:** does it. Only the effects approved get through, so build each request the same way in both.

It imports only the interfaces its capabilities declare, and a component that imports another isn't loaded. Its file system is its own folder, `/data`.

### Cards
A card's title says what the call does in a line, and its blocks show the rest: `Fields` (name and value pairs), `List`, `Diff` (files and lines changed), `Code`, and `Note` with a tone. Silt draws them, beside every effect it lets through, so a card can't look like Silt's own.

## Testing
`silt-plugin-test` is a stand-in for Silt's host. It builds the plugin, loads it as Silt does with what its manifest declares, answers its HTTP from what the test says, and records every request:

```rust
let notes = silt_plugin_test::Harness::build(env!("CARGO_MANIFEST_DIR").as_ref()).unwrap();
notes.answer("POST", "https://api.notes.example/", 201, "{}");
let call = notes.prepare("add", json!({"notebook": "work", "text": "ship it"})).unwrap();
let ran = notes.run(&call).unwrap();
assert!(ran.refused.is_empty());
```

A request the plugin didn't prepare shows in `ran.refused`, as Silt would refuse it.

## Publishing
Tag a version in the plugin's repository, and people add it with `silt plugin add <git-url>#<tag>`. Silt shows them what it gets before anything of it is built, then builds it offline in a sandbox that sees only the repository, so everything it needs must be in it or fetched by cargo: depend on Silt's crates by git, as the template does, not by path. A newer tag is offered as an update to everyone who added it at a tag, showing what it asks for that the last one didn't.

Before tagging:
- **give it your own `id` (`<publisher>.<name>`), `publisher`, `repository` and `license`;**
- **pin the `toolchain`** you build and test it with;
- **ask only for what it needs:** each capability and host with a reason that's true, every action that changes something a `change` that asks by default unless it's harmless, and cards that say exactly what each call does.
