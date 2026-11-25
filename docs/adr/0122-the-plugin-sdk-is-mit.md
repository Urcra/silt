# 0122: The plugin SDK is MIT

- Status: accepted
- Date: 2026-11-25
- Amends: [ADR 0104](0104-one-plugin-model.md) (the SDK and the template)

## Context
Silt is AGPL-3.0. A plugin written in Rust links `silt-plugin-api`, which depends on `silt-plugin` and `silt-access`, and its tests link `silt-plugin-test`, which runs it on `silt-plugin-host`. The template people start from said `MIT OR Apache-2.0`, but with every one of those crates under the AGPL, a plugin built from it was arguably a derivative of Silt and bound by the AGPL too. A team's private plugin, or a vendor's, shouldn't have to be.

## Decision
**The crates a plugin builds against are MIT:** `silt-plugin-api` with its WIT world, `silt-plugin`, `silt-access`, `silt-plugin-test` and `silt-plugin-host`. Each carries its own `LICENSE-MIT` and says `license = "MIT"` in its manifest. None of them depends on another Silt crate, so the chain stops there.

**The template is MIT too,** with a `LICENSE` of its own, so a plugin copied from it starts under a licence its author can keep or change.

**The rest of Silt stays AGPL-3.0,** including the daemon that loads plugins and the plugins shipped in `plugins/`.

## Consequences
- A plugin may be under any licence, closed ones included.
- A change to these five crates is offered under MIT; anything moved into them from the rest of Silt must be the contributor's to relicense.
- `silt-access` is also used by the daemon and the gateway; being MIT doesn't change how they use it.
