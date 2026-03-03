## For now this is just the first draft i am working on it to make it a full fleged code editor . "Just a personal project nothing more"



# TermX

TermX is a full terminal-based code editor written in Rust (`crossterm`).

It currently includes:
- Boxed editor UI with custom colors
- Startup intro message + ASCII dragon animation
- File explorer pane and editor pane
- Multi-buffer editing
- Open/create/save file and folder operations

## Build

```bash
cargo build
```

## Run

```bash
cargo run
```

Open directly with a file or folder:

```bash
cargo run -- path/to/file_or_folder
```

## Core Workflow

1. Press `F2` to open a folder.
2. Enter a path (example: `~/TermX`) and press `Enter`.
3. Press `F9` to switch focus between **Explorer** and **Editor**.
4. In Explorer, use arrows + `Enter` to open files.
5. In Editor, type and edit text.
6. Press `F5` to save.

## Keybindings (Reliable)

- `F1`: Open file (prompt)
- `F2`: Open folder (prompt)
- `F3`: Create file (prompt)
- `F4`: Create folder (prompt)
- `F5`: Save
- `F6`: Save As (prompt)
- `F7`: Close active buffer
- `F8`: Close workspace folder
- `F9`: Toggle pane focus (Explorer/Editor)
- `F10`: Quit

## Editor Controls

- `Arrow keys`: Move cursor/selection
- `Enter`: New line (or open selected item in explorer)
- `Backspace`: Delete
- `Tab`: Insert 4 spaces

Explorer-specific:
- `Up/Down`: Move selection
- `Right`: Expand selected folder
- `Left`: Collapse selected folder (or parent)

## Notes

- `Ctrl` shortcuts are also present for many actions, but some terminals intercept them. Use `F` keys if a `Ctrl` combo does not work.
- `~` and `~/...` paths are supported in prompt inputs.
