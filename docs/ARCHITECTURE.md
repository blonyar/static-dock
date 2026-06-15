# StaticDock Architecture

StaticDock is a portable static resource server with a small native Windows GUI and reproducible Windows packaging scripts.

## Runtime components

```text
src/main.rs
```

Rust HTTP server. Responsibilities:

- Parse CLI options: `--root`, `--port`, `--workers`, `--queue`.
- Serve static files with CORS, HTTP Range, ETag and Keep-Alive support.
- Expose resource-manager APIs under `/__staticdock/api/*`.
- Expose upload APIs under `/__staticdock/api/upload/*` (init, chunk, finish, status, cancel).
- Generate QR codes (SVG and BMP) via `/__staticdock/api/qr`.
- Detect usable LAN addresses via `ipconfig` parsing on Windows.
- Graceful shutdown: wait for in-flight requests on Ctrl+C (up to 30 seconds).
- Buffer reuse: each worker thread reuses a single 256 KB buffer via `thread_local!`.
- Keep request handling dependency-free and portable.

```text
gui/StaticDockGui.cs
```

C# WinForms launcher. Responsibilities:

- Choose and remember resource folders.
- Start/stop the Rust server process.
- Display local, LAN, and Android emulator URLs.
- Persist GUI settings in `%APPDATA%\StaticDock\settings.ini`.
- Keep packaging user-friendly for non-technical Windows users.

```text
resources/
```

Default mounted resource folder. It includes `index.html`, which provides the browser resource manager, and PWA assets:

- `index.html` — Single-page resource manager (browse, search, preview, upload, QR code, i18n).
- `manifest.json` — PWA manifest for "Add to Home Screen" on mobile browsers.
- `sw.js` — Service Worker that caches the app shell for offline launch; never caches API or resource data.
- `assets/` — Icons, mascot image, and showcase screenshot.

## Packaging components

```text
package/windows/
```

Single source of truth for Windows package launcher scripts copied into `dist/` by `build-windows.bat`.

```text
build-windows.bat
```

Local Windows release build entrypoint:

1. Build Rust release binary into `target/release/`.
2. Copy build executables into `build/windows/`.
3. Build WinForms GUI with .NET Framework `csc.exe` into `build/windows/`.
4. Create standard and loadtest distribution folders.
5. Generate zip packages.

```text
scripts/release/
```

Release automation and package checks.

```text
tools/loadtest/
```

Loadtest runner scripts copied into the full release package.

```text
loadtest/
```

Loadtest data set used by the full release package.

## HTTP API

All API endpoints live under `/__staticdock/api/` and return JSON unless noted.

```text
GET  /api/list?path=/         List directory entries (name, url, kind, bytes, modified)
GET  /api/info                Server info (name, root path)
GET  /api/qr?text=URL&format=svg|bmp   QR code image
POST /api/upload/init         Create upload session (path, filename, size, mime → id, chunk_size)
POST /api/upload/chunk        Upload a 1 MB chunk (X-Upload-Id, X-Chunk-Offset headers)
POST /api/upload/finish       Merge chunks into final file (id → filename, bytes)
GET  /api/upload/status?id=X  Query upload progress (received_bytes, total_size)
POST /api/upload/cancel       Cancel and clean up upload session (id)
```

Upload sessions are stored under `<root>/.staticdock-uploads/<id>/` and expire after 24 hours.

## Request lifecycle

1. Main thread accepts TCP connections and dispatches to worker threads via a bounded channel.
2. Each worker reads the HTTP request, routes to a handler (static file, API, or upload).
3. Static files: resolve path, check root boundary, apply ETag/If-None-Match, handle Range, stream with `copy_limited`.
4. Keep-Alive: connections are reused for multiple requests until the client closes or idle timeout (30 s).
5. Graceful shutdown: `Ctrl+C` sets a global `AtomicBool`; the accept loop stops; workers finish current requests.

## Generated local artifacts

These are intentionally ignored and should not be committed:

```text
build/windows/
static-dock-urls.txt
static-dock-server.pid
dist/
target/
```

Release zip files are generated under `dist/` and uploaded to GitHub Releases, not kept as source files.

## Directory policy

- Source code lives in `src/` and `gui/`.
- User-facing default content lives in `resources/`.
- Package launchers live in `package/windows/`.
- Automation lives in `scripts/`.
- Developer tools live in `tools/`.
- Build outputs stay out of the repository root and out of Git.
