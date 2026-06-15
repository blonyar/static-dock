# Changelog

All notable changes to StaticDock are documented here.

## v0.3.0

### Performance
- **HTTP Keep-Alive**: Connections are reused for multiple requests, dramatically reducing latency for directory browsing and batch file loading.
- **Buffer reuse**: Each worker thread now reuses a single 256 KB buffer via `thread_local!`, eliminating per-request heap allocations.
- **ETag + 304 Not Modified**: Files now return `ETag` headers; repeat requests for unchanged files get a `304` response with no body, saving bandwidth.
- **Cached canonicalize**: `ensure_inside_root` no longer re-canonicalizes the root path on every request.
- **LAN detection via `ipconfig`**: Replaced the PowerShell-based LAN IP detection with direct `ipconfig` parsing, reducing startup time by 1–3 seconds.

### Features
- **File upload with chunked transfer**: Upload files via the WebUI with drag-and-drop or the upload button. Large files are split into 1 MB chunks. Upload progress and speed are displayed in real time.
- **LAN and upload safety controls**: The server accepts `--bind`, `--no-upload`, and `--uploads`; the GUI can switch between local-only and LAN access and can disable browser uploads for read-only sharing.
- **PWA (Progressive Web App)**: `manifest.json` and a Service Worker enable "Add to Home Screen" on iOS and Android. The app shell is cached for instant re-launch and works offline for the UI shell.
- **Mobile-optimized UI**: Fixed bottom navigation bar (≥ 44 px touch targets), safe-area-inset support for notched screens, and larger file thumbnails on phones.
- **Graceful shutdown**: Pressing Ctrl+C now waits for in-flight requests to complete (up to 30 seconds) before exiting, preventing truncated downloads.

### GUI
- Replaced `Thread.Sleep(800)` with `WaitForExit(800)` for non-blocking server startup detection.
- Empty `catch {}` blocks replaced with logged warnings for better diagnostics.
- Added `SetProcessDpiAwarenessContext` for Per-Monitor V2 DPI support on Windows 10+.

### Packages

- `static-dock-windows-x64.zip`: recommended for most users.
- `static-dock-windows-x64-with-loadtest.zip`: includes loadtest assets and scripts.

## v0.2.8

### Highlights

- Added an isolated native GUI settings mode for portable runs and repeatable verification: `StaticDockGui.exe --settings-dir <folder>`.
- Added `STATICDOCK_SETTINGS_DIR` and `StaticDock.portable` support for users who do not want GUI settings to be stored under `%APPDATA%`.
- Improved GUI startup diagnostics by logging the active settings file, server executable, mounted folder, launch command, and actual port.

### Packages

- `static-dock-windows-x64.zip`: recommended for most users; smaller package without loadtest assets.
- `static-dock-windows-x64-with-loadtest.zip`: includes loadtest assets and scripts for local stress testing.

## v0.2.7

### Highlights

- Fixed native Windows GUI server startup for shared folders whose paths contain spaces, Unicode characters, or common directories such as Downloads.
- Rebuilt and validated the Windows standard and loadtest packages after the path quoting fix.

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

## v0.2.6

### Highlights

- Fixed phone QR code loading in the native Windows GUI by serving a WinForms-compatible BMP QR endpoint while keeping SVG QR output for browsers.
- Improved the WebUI resource manager with modal-first preview/open behavior, richer file-type icons, and right-click actions for preview, new-tab open, download, and URL copy.
- Improved video, audio, and PDF preview handling with embedded controls and clear fallback guidance when the browser cannot decode a file inline.
- Rebuilt and validated the Windows standard and loadtest packages.

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
