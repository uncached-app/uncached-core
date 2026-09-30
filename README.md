# uncached-core

The Rust library behind [Uncached](https://uncached.app): it finds the folders a
toolchain can rebuild, says how sure it is about each one, and removes them.
The desktop app ([`uncached-desktop`](https://github.com/uncached-app/uncached-desktop))
depends on it by git revision.

## What is in it

| Module | Responsibility |
| :--- | :--- |
| `scanner` | Walks a workspace and asks each provider whether a folder is a target. A folder counts only when the project file beside it confirms it: `target` next to a `Cargo.toml` or a `pom.xml`, never by name alone. |
| `discovery` | Machine-wide locations: package manager, AI tool and editor caches, and the known system paths. |
| `confidence` | Turns the signals behind a target into a level (high, medium, review) and the reasons shown to the user. |
| `cleaner` | Validates a target again at the moment of removal, then moves it to the Trash or deletes it permanently. |
| `dependencies` | Declared dependencies nothing in a project names, for `package.json` and `Cargo.toml`, with the command that removes each. |
| `ignore_rules` | User rules that keep paths out of a scan. |
| `contract` | The versioned types the app receives over IPC. |

## Development

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo run --example probe-home        # what discovery finds on this machine
cargo run --example assess-path -- <path>
```

The desktop app builds against a sibling checkout of this repository through a
`[patch]` in its `src-tauri/.cargo/config.toml`, so a change here is picked up
there without moving the pinned revision. A release pins the revision it was
built with.

## License

Proprietary.
