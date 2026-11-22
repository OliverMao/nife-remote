# Building

[← Back to README](../README.md) · [简体中文](building.zh-CN.md)

**The client is Windows only** (the macOS / Linux implementations have been removed). Which account service the client talks to is decided **at build time** (see [Account service address](#account-service-address)), so a release build needs one; the relay address and fingerprint are handed out by it and are not compiled in.

## Desktop client

### Requirements

| Needs | Notes |
|---|---|
| Windows 10 / 11 | WebView2 usually ships with the OS; if it's missing the installer downloads and installs it |
| Rust | A recent stable (edition 2024) |
| Node.js 20+ | Builds the console UI |
| CMake | Builds libopus |

### Account service address

Where the client connects is decided **at build time only** — neither the UI nor the configuration file can change it,
and the UI never shows it. `client/src-tauri/build.rs` reads it from the environment variable `NIFE_REMOTE_AUTH`, then
`client/.env`, then falls back to the official service `https://nife-remote-auth.naig.us`. A plain build uses the default:

```powershell
npm run tauri build
```

If you run your own account service (see [deployment](../deploy/deploy.md)), set the variable or put it in `client/.env`
(copy `client/.env.example`) before building:

```powershell
$env:NIFE_REMOTE_AUTH="https://auth.example.com"     # proxy domain, no port
npm run tauri build
```

The relay address and fingerprint are not compiled in: the account service hands them out with `GET /relay` and the client follows (see [Configuration file](usage.md#configuration-file)).

### Build

Windows (PowerShell):

```powershell
cd client
npm install
npm run tauri build            # exe only: npm run tauri build -- --no-bundle
```

The relay doesn't have to be compiled in (the account service assigns it, see [Account service address](#account-service-address)).

The output is under `client/src-tauri/target/release/`:

| File | Notes |
|---|---|
| `bundle/nsis/Nife Remote_<version>_x64-setup.exe` | The installer |
| `nife-remote.exe` | What `--no-bundle` produces (see the Windows notes) |

### Windows notes

- The installer installs for all users and registers the system service (see [Windows installation](usage.md#windows-installation)); its extra steps are in `client/src-tauri/windows/hooks.nsh`
- `npm run tauri build -- --no-bundle` only builds `nife-remote.exe` (no NSIS), but a release build refuses to run until installed. To install it without the installer (administrator):

  ```powershell
  mkdir "C:\Program Files\Nife Remote"
  copy .\client\src-tauri\target\release\nife-remote.exe "C:\Program Files\Nife Remote\"
  & "C:\Program Files\Nife Remote\nife-remote.exe" --install     # registers and starts the service; --uninstall removes it
  ```

  Once installed the exe can't be moved (the service records that path)
- Build speed: `[profile.release]` uses `lto = false` + `codegen-units = 256` (parallel). The old `lto = "fat"` + `codegen-units = 1` left the last step single-threaded and took minutes; a rebuild after a one-line change is now about 45 s, at the cost of roughly 3 MB of binary size
- The C runtime is statically linked, so the VC++ redistributable isn't needed
- Media Foundation is delay-loaded, so Windows N editions (without the Media Feature Pack) still run and use software encoding
