# Changelog

All notable changes to StaticDock are documented here.

## v0.2.5

### Highlights

- Improved LAN address detection by filtering virtual/test ranges such as `198.18.x.x` and `198.19.x.x`, making the displayed phone/LAN URLs more reliable.
- Added recent resource directory history in the native WinForms GUI for faster switching between commonly used folders.
- Kept the Windows no-dependency experience: native C# WinForms launcher, default `resources/`, default port `8787`, and the built-in resource manager.
- Improved release engineering with reproducible Windows package generation, release package checks, changelog-based notes, and GitHub Actions publishing.

### Packages

- `static-dock-windows-x64.zip`: recommended for most users; smaller package without loadtest assets.
- `static-dock-windows-x64-with-loadtest.zip`: includes loadtest assets and scripts for local stress testing.

### Quick start

Extract the package, then double-click:

```text
start-static-dock-gui.bat
```

Default URL:

```text
http://127.0.0.1:8787/
```
