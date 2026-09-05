# Lotus Photo Mode

`lotus-photo` renders one isolated native Lotus scene from a small JSON file. It does not
launch apps, change normal Lotus, read settings, install hooks, or enumerate windows.

```powershell
cargo run -p lotus-photo -- crates/lotus-photo/scenes/dock.json
cargo run -p lotus-photo -- --validate crates/lotus-photo/scenes/search.json
```

The schema is an object with `kind` (`dock`, `search`, or `switcher`), optional `dpi` (96..384,
default 192), nonempty ordered `apps`, optional `query`, and optional `selected` index. Each app
has a display `name` and an executable or shortcut `path`; relative paths are resolved from the
scene file directory. Installed shortcuts and executables provide the real application icons.
The included scenes pick Zen, Steam, OBS Studio and File Explorer from this PC; edit the paths
for another machine. At 192 DPI, the whole scene is rendered at twice its normal size.

Close the photo window with `Alt+F4`.
