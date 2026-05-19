# matrix-account-tools

A terminal UI for managing your Matrix account.

*Disclaimer:* Built with the help of AI (Claude Sonnet 4.6)

## Tools

| Command | Description |
|---|---|
| `:leaverooms` | Browse joined rooms, select multiple, and leave them in parallel |
| `:rooms` | Browse joined rooms with filter (`/`) and per-room detail view |
| `:accounts` | Switch between or remove saved accounts |
| `:ignorelist` | View, add, and remove ignored users |
| `:profile` | Edit your display name and avatar URL |
| `:devices` | View logged-in devices and sign out others |

## Usage

```
cargo run
```

On first launch you'll be prompted to log in. Sessions are persisted and restored automatically on next start. Background sync keeps room and account data fresh without blocking the UI.

## Keybindings

`?` opens the full keybinding reference from any screen.
