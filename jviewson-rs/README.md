# JviewSON RS

Rust rewrite of JviewSON using `eframe/egui`.

## Features
- Open `.json` files
- Tree view (expand/collapse objects and arrays)
- Text view (read-only formatted JSON)
- Search highlighting in tree labels
- Drag-and-drop file loading
- Optional auto-reload when the source file changes

## Build

Linux (dev):
```bash
cargo run
```

Windows `.exe` (from this Linux/WSL environment):
```bash
cargo build --release --target x86_64-pc-windows-gnu
```

Output:
- `target/x86_64-pc-windows-gnu/release/jviewson-rs.exe`
- `dist/windows/jviewson-rs.exe`
