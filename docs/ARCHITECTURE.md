# StaticDock Architecture

StaticDock is a portable static resource server with a small native Windows GUI and reproducible Windows packaging scripts.

## Runtime components

```text
src/main.rs
```

Rust HTTP server. Responsibilities:

- Parse CLI options: `--root`, `--port`, `--workers`, `--queue`.
- Serve static files with CORS and HTTP Range support.
- Expose resource-manager APIs under `/__staticdock/api/*`.
- Detect usable LAN addresses for terminal output.
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

Default mounted resource folder. It includes `index.html`, which provides the browser resource manager.

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
