# matrix-account-tools

A terminal UI for managing your Matrix account.

*Disclaimer:* Built with the help of AI (Claude Sonnet 4.6)

## Tools

### `:rooms` — Room browser

- Filterable room list with encryption and DM indicators
- Room detail: name, alias, topic (editable), member count, last activity
- Member list with power levels, kick, ban, and ignore actions
- Leave-select mode: toggle multiple rooms and leave them in parallel

### `:accounts` — Account manager

- Switch between saved accounts or add a new one
- Profile: edit display name and avatar URL
- Devices tab: view logged-in sessions, sign out others
- Ignored users tab: view, add, and remove ignored users

## Usage

```
cargo run
```

On first launch you'll be prompted to log in with your Matrix user ID and password. Sessions are persisted and restored automatically. Background sync keeps room and account data fresh.

Press `?` at any screen for the full keybinding reference, or `:` to open the command bar.
