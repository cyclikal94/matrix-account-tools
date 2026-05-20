# matrix-account-tools

A terminal UI for managing your Matrix account.

*Disclaimer:* Built with the help of AI (Claude Sonnet 4.6)

<img width="1002" height="527" alt="image" src="https://github.com/user-attachments/assets/0fe945f5-f8b6-4e2e-8e6a-aeead8008b8c" />

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

## Installation

Download the latest binary for your platform from [Releases](https://github.com/cyclikal94/matrix-account-tools/releases).

| Platform | File |
|---|---|
| Linux x86_64 | `matrix-account-tools-x86_64-linux` |
| macOS Intel | `matrix-account-tools-x86_64-macos` |
| macOS Apple Silicon | `matrix-account-tools-aarch64-macos` |
| Windows x86_64 | `matrix-account-tools-x86_64-windows.exe` |

**macOS / Linux:**

```sh
chmod +x matrix-account-tools-*
./matrix-account-tools-aarch64-macos   # or whichever matches your platform
```

On macOS, the binary is unsigned. If Gatekeeper blocks it, run once with:

```sh
xattr -d com.apple.quarantine matrix-account-tools-aarch64-macos
```

## Data storage

Sessions and room cache are stored in the platform config directory:

| Platform | Path |
|---|---|
| macOS | `~/Library/Application Support/matrix-account-tools/` |
| Linux | `~/.config/matrix-account-tools/` |
| Windows | `%APPDATA%\matrix-account-tools\` |

- `accounts.json` — saved accounts (homeserver, session tokens)
- `stores/<user>_at_<homeserver>/` — per-account SQLite database (room cache, crypto store)

## Building from source

```
cargo build --release
```

## Usage

On first launch you'll be prompted to log in with your Matrix user ID and password. Sessions are persisted and restored automatically. Background sync keeps room and account data fresh.

Press `?` at any screen for the full keybinding reference, or `:` to open the command bar.
