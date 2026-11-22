# Notes

An in-game notepad for [Horizon](https://github.com/horizon-rust/horizon). Jot down coordinates, to-do lists or server info without leaving Minecraft, and pin the important ones on your HUD.

Client-side only, written in Rust (WebAssembly). Nothing to install on servers.

## Features

- **Notepad screen** opened with **N** (rebindable in Options > Controls).
- **Create, edit, delete and browse** notes. Each note has a unique ID, a title and a multi-line body with word wrap, cursor movement and text selection.
- **Three categories**, picked with tabs:
  - **WORLD**: notes tied to one singleplayer world (by save folder).
  - **SERVER**: notes tied to one multiplayer server (by address).
  - **GLOBAL**: notes available everywhere.
- **Automatic context**: the mod detects the world or server you are in and shows the matching notes.
- **HUD pinning**: pin a note to show it in the top-left corner (up to 3 at once). It updates live when you edit the note and disappears when you unpin it.
- **Safe local saving**: notes are written atomically on every change, persist across restarts, world changes and disconnects, and are never overwritten if a file can't be read (unreadable lines are copied to a `.bak` file).

## Installation

1. Download `Notes.hmod`.
2. Put it in your Horizon mods folder (or install it through the launcher).
3. Launch the game and press **N** in a world or on a server.

## Usage

| Action | How |
| --- | --- |
| Open / close | **N** / **Esc** |
| New note | **New** button |
| Select a note | Click it in the list |
| Switch title / body | Click, or **Tab** |
| Pin / unpin | **Pin** / **Unpin** button |
| Delete | **Delete**, then **Confirm?** |
| Switch category | **WORLD**, **SERVER** or **GLOBAL** tab (only the ones that apply to where you are are enabled) |

Notes are stored in `horizon-mod-data/notes/` as plain text (`global.txt`, `world-*.txt`, `server-*.txt`).

## Limitations

- No creation or modification dates: the mod API exposes no wall-clock time.
- No copy/paste or mouse-wheel scrolling in the note body.
- The screen can't open over another screen or from the main menu, and can't be used with a controller.
- Servers are identified by address, not by display name.
- UI text is English only.

## Building

Requires the Horizon mod toolchain (Rust 1.99.1 with the `wasm32-unknown-unknown` target and `rust-src`, plus `cargo-horizon-mod`):

```sh
cargo horizon-mod build --wasm
# -> target/horizon-mod/notes.hmod
cargo test   # unit tests
```

## License

MIT
