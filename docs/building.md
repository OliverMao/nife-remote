# Building

[← Back to README](../README.md) · [简体中文](building.zh-CN.md)

**The client is Windows only** (the macOS / Linux implementations are not built or supported). The client does **not** need a relay server to build: the address is filled in the installed app, so there is no need to deploy the server first. To ship a preconfigured installer, compile the address and fingerprint in as defaults (see [Server address and certificate fingerprint](#server-address-and-certificate-fingerprint)).

## Desktop client

### Requirements

| Needs | Notes |
|---|---|
| Windows 10 / 11 | WebView2 usually ships with the OS; if it's missing the installer downloads and installs it |
| Rust | A recent stable (edition 2024) |
| Node.js 20+ | Builds the console UI |
| CMake | Builds libopus |

### Server address and certificate fingerprint

The client does **not** need the server at build time: leave both values out, build, and set them in the installed client (server indicator → **Change** → address + optional fingerprint → **Save and restart**).

To ship a preconfigured installer, bake the defaults in at build time with the `NIFE_REMOTE_SERVER` and `NIFE_REMOTE_SERVER_FP` environment variables, or change the defaults in `client/src-tauri/src/config.rs`:

- Without `NIFE_REMOTE_SERVER` the client starts with `127.0.0.1:4433` until one is set in the app
- A malformed fingerprint (it must be 64 hex digits; `:` and spaces are allowed) fails the build
- These are only defaults: once installed, `server` and `server_fp` in `config.toml` override them (see [Configuration file](usage.md#configuration-file)). An invalid `server_fp` there refuses every connection rather than turning verification off

> [!NOTE]
> When `NIFE_REMOTE_SERVER_FP` is empty the client remembers the server's certificate on its first connection (`[server_trust]` in `config.toml`) and refuses a different one later (trust on first use; the console shows "Trust on first use" at the top left). Only that first connection is unprotected against a man in the middle, so pin the fingerprint for releases. If the server's certificate changes on purpose, delete `[server_trust]` from `config.toml`.

### Build

Windows (PowerShell):

```powershell
cd client
npm install
npm run tauri build            # exe only: npm run tauri build -- --no-bundle
```

The server address doesn't have to be compiled in (it is set in the installed client, see [Server address and certificate fingerprint](#server-address-and-certificate-fingerprint)).

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
