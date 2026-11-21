# Development

[← Back to README](../README.md) · [简体中文](development.zh-CN.md)

## Tests

```bash
# repository root: protocol + link library + server (debug builds check the order of every server lock, see server/core/src/broker/lock.rs)
cargo test
# the server only uses nife-remote-link's wire module (default client feature off); the cargo test above always builds it with client, so check this separately
cargo test -p nife-remote-link --no-default-features

# crates/nife-remote-proto/fuzz/: what the server parses before any authentication (version handshake, signaling messages, session headers); needs nightly and cargo-fuzz
cargo +nightly fuzz run signal    # also hello, session_header

# web/: web console (WebSocket protocol, password, HTTPS certificate)
cargo test

# client/: UI (jitter buffer and A/V sync simulations, H.264 conversion, UI strings)
npm test

# client/src-tauri/: client Rust (encoders incl. hardware probing, Opus packing, format conversion…)
cargo test --lib
# audio loopback capture (needs an audio output device; plays a 1.5 s test tone at -60 dBFS, barely audible)
cargo test --lib loopback_captures_tone -- --ignored --nocapture
# Windows: the controller's microphone into Steam Streaming Microphone (needs it on this computer), recorded back as a 1 kHz tone
cargo test --lib mic_reaches_steam_streaming_microphone -- --ignored --nocapture
```


End to end (a real server, a real host with screen capture and encoding, and a protocol-level controller):

```bash
# repository root: start a server without certificate pinning
cargo build -p nife-remote-server
NIFE_REMOTE_CERT=target/e2e/cert.pem NIFE_REMOTE_KEY=target/e2e/key.pem ./target/debug/nife-remote-server &

# client/src-tauri/
cargo test --lib e2e_roundtrip -- --ignored --nocapture   # video / codec switching / clipboard / audio / ping / disconnect
cargo test --lib e2e_reconnect -- --ignored --nocapture   # getting the code back after a server restart, changing the code
cargo test --lib e2e_mic -- --ignored --nocapture         # Windows: the controller's microphone through the relay into Steam Streaming Microphone
```

## Debugging tools

```bash
# client/: debug build (the console opens at startup; the host runs in the same process); NIFE_REMOTE_LOG sets the log level
NIFE_REMOTE_LOG=debug npm run tauri dev
# client/: A/V sync measurement (the host plays a white flash + short beep; usage at the top of the script)
node scripts/measure-avsync.mjs <control-code> [pairing-code]

# client/src-tauri/: headless host, for testing the controller on the same machine
# (mute the controller, otherwise the played-back audio is captured again and echoes)
cargo run --release --example headless_host
# client/src-tauri/: encoder benchmark (captures the real screen and compares the encoders)
cargo run --release --example encode_bench
```

Use release builds when judging smoothness or A/V sync: CPU scaling is very slow in debug builds (used for "Sharp text" or when the GPU can't scale; 4K → 1080p takes about 0.5 s per frame). To compare the GPU zero-copy and CPU paths: `cargo run --release --example encode_bench`.

## Debugging the Windows service

Debug builds run the host inside the tray process, so `npm run tauri dev` needs no installation. To work on the service itself, install the installer, or register a build directly from an administrator terminal:

```powershell
# client/src-tauri/
target\release\nife-remote.exe --install       # register and start the service for this nife-remote.exe (creates config.toml next to it)
target\release\nife-remote.exe --stop-service  # stop it before rebuilding: the exe is in use
target\release\nife-remote.exe --uninstall     # remove the service and the startup entry
```

The service and the host log with `OutputDebugString`: run Sysinternals DebugView as administrator and enable Capture → Capture Global Win32. For a different log level, add `NIFE_REMOTE_LOG=debug` to the multi-string value `Environment` under `HKLM\SYSTEM\CurrentControlSet\Services\Nife Remote` and restart the service. While the service is installed, a debug build started from elsewhere still runs its own in-process host (with its own `config.toml` and control code).

## Web console

```bash
# build the UI, then start the service in web/ (connects to a server on 127.0.0.1:4433, serves ../client/dist); open http://localhost:8080
(cd client && npm run build)
cd web && cargo run
# while working on the UI: npm run dev in client/ and open http://localhost:1420 (/ws is proxied to nife-remote-web on port 8080)
```

The WebSocket protocol between the console and the controller service (the same for the desktop console and the browser) is described at the top of `crates/nife-remote-link/src/console.rs`.

## Conventions

### Protocol versions

There are two version numbers (`crates/nife-remote-proto/src/lib.rs`); the rules are in [How it works → Protocol evolution](architecture.md#protocol-evolution):

- `SIGNAL_VERSION` / `SIGNAL_MIN`: client ⇄ server (signaling, the session header, the datagram's session ID). The server accepts clients within its range; without a common version it replies `Incompatible` (with its own range) and the client tells the user whether to update Nife Remote or the server, then retries once a minute
- `SESSION_VERSION` / `SESSION_MIN`: host ⇄ controller, negotiated in the end-to-end handshake; the server never looks at it. Without a common version the controller shows `peerIncompatible`

Before changing the protocol, look for a compatible way; if there is one, don't bump a version:

- New message: add an enum variant with a **new number**; older versions skip it. If the peer may not know it, add a capability bit in `features` and only send it when the peer advertises the bit
- New field: only `Option` fields (or `#[cbor(default)]`), with a new number; required fields must stay forever
- New enum values: enums that are just a number are declared with `open_enum!` (`crate::cbor`), so an unknown value decodes as `Unknown(n)` wherever it appears; decide what the receiver does with it (ignore it, fall back to a default, treat it as "the peer is newer")
- New variants of enums with fields: they can only be skipped as a whole message, in an `Option` field marked `crate::cbor::opt`, or as a list element marked `crate::cbor::open_vec`. Anywhere else an unknown variant is a decoding error, on purpose. Never put an enum with fields inside a plain derived `Option<…>` (also not inside a struct that sits in an `Option`): minicbor's derive treats any unknown variant inside as `None` and skips from the wrong position (see the notes in `cbor.rs`)
- Numbers can't be changed or reused once released; bump the version only when there's no other way (e.g. removing a required field)

The version range in the version handshake (keys 0 and 1 of the `ClientHello` / `ServerHello` CBOR maps, `signal::Versions`) and the ALPN must **never** change, otherwise different versions can't even tell each other that they differ. The `version_handshake_is_frozen` test guards this. The `nife-remote-proto` tests include examples of a newer peer sending new messages, fields and enum values; write one like them when adding a compatibility rule.

### One protocol, two implementations

`crates/nife-remote-proto/tests/v9.rs` holds the v9 CBOR bytes. Once v9 is released they are fixed and must not be regenerated from the implementation: existing message numbers and field positions stay compatible, and new messages get separate fixtures. Cursor receiver tests cover ordering across both lanes, and a server test uses real QUIC and the controller session to verify that unknown video formats are acknowledged and subsequent frames still arrive.

The Rust side and the console (TypeScript) speak the same formats: media packets (`crates/nife-remote-proto/src/media.rs`), input events, UI commands and connect options (`nife_remote_link::controller::Command`, `ConnectOptions`). All the JSON between the UIs and Rust is camelCase: field names, enum values (`"h264"`, `"android"`, `"quickSettings"`), and a field naming the variant of tagged unions (`cmd` for commands, `type` for events and input, `kind` for problems and notices). `crates/nife-remote-link/tests/protocol.json` holds samples of each, generated by a Rust test; `npm test` (`tests/protocol.test.ts`) checks its parser and builders against the same file. After changing a format, regenerate it with `NIFE_REMOTE_UPDATE_FIXTURES=1 cargo test -p nife-remote-link --test protocol`, then make the TypeScript tests pass.

`client/tests/protocol-contract.ts` contains literals generated by the same Rust test. `satisfies` checks every field against the actual TypeScript types, and `npm test` runs `tsc` first. A type annotation on `JSON.parse()` cannot replace this check because its `any` result bypasses validation.

### Messages are types, not text

Anything the user may see that crosses the network or comes from the Rust side is an enum (`ConnectError`, `Notice`, `ByeReason`, `Problem`…). The UI turns it into text in the user's language:

- Console strings live in `client/src/locales/zh.ts` (the full list of keys) and `en.ts`
- Tray strings use `tr!` in `client/src-tauri/src/tray.rs`
- `npm test` fails if a Rust `Problem` / `NoticeView` variant has no string in either language
- Logs and internal errors are English

### Icon

The only source is `assets/icon.svg`. After editing it run `node scripts/icon.mjs` from the repo root: it renders the SVG with the system Edge/Chrome (it uses foreignObject, which plain rasterizers drop) and regenerates the app icons, the tray icon, the console logo and the favicon. See `assets/README.md`.
