# Deployment: the four sides

[← Back to README](../README.md) · [简体中文](deploy.zh-CN.md)

How to put Airhand into a real environment: where each side goes, which ports to open, what to configure. To get something running first, see the [quick start](../README.md#quick-start); this page takes each side apart.

## The sides

| Side | What it is | Where it goes | Ports |
|---|---|---|---|
| **Relay server** | Matches the two ends by control code and forwards encrypted bytes only | A machine with a public IP, Docker | inbound **UDP 4433** |
| **Host** | Captures screen / sound, encodes, injects input | Every computer to be controlled | outbound UDP only |
| **Controller (desktop)** | Decodes, displays, captures input | Your own computer | outbound UDP only |
| **Web console** | The controller running on a server, browser as its UI | Any machine with Docker | inbound **TCP 8080** |

The host and the controller are the **same installer**: switch roles from the tray menu, no second install. The web console is another form of the controller; the controlled computers still run the client.

```
                    ┌──────────────────┐
   host    ──UDP──▶ │  relay server    │ ◀──UDP── controller (desktop)
 (screen source)    │  UDP 4433        │
                    └──────────────────┘ ◀──UDP── web console (container)
                             ▲
                             └── browser ──TCP 8080──▶ web console
```

When the two ends can punch through, picture and sound skip the server (see [Direct path and ports](#direct-path-and-ports)); the server still matches them up and carries the fallback copy.

## Relay server

The server listens on **UDP 4433**. On first start it generates a self-signed certificate in the `data/` directory next to `docker-compose.yml`. **Keep that directory**: clients recognise the server by the certificate's fingerprint, and it holds the SQLite database with the hosts' fixed control codes (`airhand.db`, plus its `-wal` / `-shm` files while the server runs). Every host that keeps its code gets the same code back from it, however long it has been offline.

### Start it

```bash
git clone <repository URL> airhand
cd airhand
docker compose up -d --build
docker compose logs airhand | grep fingerprint
```

The log prints the certificate's SHA-256 fingerprint (64 hex digits) that clients need. Check that it runs:

```bash
docker compose ps
ss -lunp | grep 4433        # should be 0.0.0.0:4433
```

The image is based on distroless, about 28 MB, with no shell: **logs are all you get**, `docker exec` doesn't work.

### Environment (see `docker-compose.yml`)

| Variable | Default | Meaning |
|---|---|---|
| `AIRHAND_LISTEN` | `0.0.0.0:4433` | Listen address (UDP) |
| `AIRHAND_CERT` / `AIRHAND_KEY` | `data/cert.pem` / `data/key.pem` next to the server binary (`/data/cert.pem` / `/data/key.pem` in the image) | Certificate paths, generated when missing; `data/` is created if needed |
| `AIRHAND_CERT_NAMES` | `airhand,localhost` | SANs written into a generated certificate |
| `AIRHAND_DB` | `airhand.db` next to the certificate (`data/airhand.db` by default) | SQLite database with the fixed control codes (codes, hashes of host secrets, last seen). Lose it and an offline host's code may go to someone else (it can still be reclaimed while nobody holds it). If it can't be opened, codes are kept in memory and an error is logged |
| `AIRHAND_MAX_SESSIONS` | `4` | Controllers allowed at once per host |
| `AIRHAND_MAX_CONNS_PER_IP` | `128` | Simultaneous connections per IPv4 address / IPv6 /64 (one per web console tab; raise it when many hosts share one egress IP) |
| `AIRHAND_NEW_CODES_PER_IP` | `30` | New codes handed out per IPv4 address / IPv6 /48 per 10 minutes (reclaiming a code doesn't count) |
| `AIRHAND_CONNECTS_PER_MINUTE` | `6` | Connection attempts per IPv4 address / IPv6 /64 to the same host per minute (never below `AIRHAND_MAX_SESSIONS`; raise it when many controllers share one egress IP) |
| `RUST_LOG` | `info` | Log level |

The container runs as root, so a mounted `data/` is writable whatever it belongs to (Docker creates it as root when missing). With rootful Docker on Linux the files it writes belong to root: read, back up and delete them with sudo.

### UDP buffers

The server sets its socket buffers to 8 MiB (the system default is about 208 KB; one burst of a tens-of-Mbps picture fills that and drops a whole batch, which shows up as stutter and latency spikes). Linux silently caps anything above `net.core.rmem_max` / `wmem_max` (you can't change that inside a container, only on the host), and the server warns at startup when it was capped:

```bash
sudo sysctl -w net.core.rmem_max=8388608 net.core.wmem_max=8388608
# survive reboots:
echo -e 'net.core.rmem_max=8388608\nnet.core.wmem_max=8388608' | sudo tee /etc/sysctl.d/90-airhand.conf
```

### Monitoring

There is no HTTP health endpoint; watch three things:

```bash
docker compose ps                 # is the container up
ss -lunp | grep 4433             # is the UDP port bound
docker compose logs --tail=100 airhand
```

## Host

### Install

| System | How |
|---|---|
| Windows 10 / 11 | Run `Airhand_<version>_x64-setup.exe` as administrator; installs to `C:\Program Files\Airhand` |
| macOS 13+ | Open the dmg, drag `Airhand.app` into Applications, grant **Screen & System Audio Recording** and **Accessibility** on first launch |
| Linux | `sudo apt install ./Airhand_<version>_amd64.deb` (X11 desktops only) |

On Windows the host runs as the system service **Airhand** (SYSTEM), so the lock screen, the sign-in screen and UAC prompts can be controlled, and it follows user switches. The tray icon is a normal program that shows the service state and operates it. An uninstalled `airhand.exe` can't be run directly: release builds ask for the installer.

On macOS the host lives in the menu bar and only appears in the Dock while the console is open. After granting Screen & System Audio Recording, reopen Airhand.

The Linux tray uses AppIndicator; stock GNOME needs the “AppIndicator and KStatusNotifierItem Support” extension. Without the library the program still runs as a host and prints its control code to stderr.

### Point it at your server

Two ways, pick one:

1. **Baked in at build time** (best for mass deployment): set `AIRHAND_SERVER` and `AIRHAND_SERVER_FP` when building the client, so a fresh install is already correct. See [Building](building.md#server-address-and-certificate-fingerprint).
2. **Edit `config.toml` after installing** (simplest for a few machines):

| System | Location |
|---|---|
| Windows | `C:\Program Files\Airhand\config.toml` (edit as administrator) |
| macOS | `~/Library/Application Support/app.airhand.client/config.toml` |
| Linux | `~/.config/airhand/config.toml` |

```toml
server = "relay.example.com:4433"
server_fp = "<64 hex digits>"
```

An empty `server_fp` also works: the server's certificate is remembered on the first connection (TOFU) and a different one is refused later, but that first connection isn't protected against a man in the middle, so pin it for real deployments. Restart Airhand afterwards (restart the service / quit and reopen from the tray on Windows).

### Security and unattended use

- `access = "pairing"` by default: a controller that hasn't paired enters the 6-digit pairing code from the tray on its first connection (it never goes through the server). Only consider `access = "open"` on a relay you fully trust: it means trusting the relay
- Control codes are **fixed** and stored in `config.toml`: they survive restarts, upgrades and relay restarts until you **change the control code**. An offline host keeps its code for 180 days
- Windows starts the host at boot by default (the service starts before sign-in, so the machine can be controlled before anyone logs in); macOS has “Open at login” in the menu
- A host needs **no inbound port**: only outbound UDP to the relay

## Controller (desktop)

The same installer, role switched from the tray:

- Windows: tray → **Open console** (the host in the service keeps running while the console is open)
- macOS / Linux: tray → **Switch to controller**

`server` / `server_fp` in `config.toml` are the same as for a host; the console's own identity is in `[controller]`, remembered computers in `[[device]]`. On Windows these are encrypted per account (DPAPI), one set per user.

A controller also needs **outbound UDP only**. Tabs, multi-screen overview, quality, sound and clipboard are usage topics: see the [usage guide](usage.md#controller).

## Web console (Docker)

The controller can also run in a container: start it with Docker Compose and a browser port is the console, with nothing to install or build on the computer you operate from. The UI is exactly the desktop console (multiple sessions, multi-screen overview, quality, sound, clipboard…); the controlled computers still run the Airhand client.

```bash
cd web
AIRHAND_SERVER=relay.example.com:4433 AIRHAND_SERVER_FP=<relay certificate fingerprint> docker compose up -d
# or cp .env.example .env, edit it and just docker compose up -d
```

Then open:

- On the machine running the container: `http://localhost:8080`
- From other computers: `https://<that machine's address>:8080`

One port serves both HTTP and HTTPS. Browsers only offer WebCodecs video decoding on secure pages (HTTPS or localhost), so use https:// from other computers (an http:// page links to the https one). The certificate is self-signed and generated on first start into `web/data/` (a bind mount: keep it and the browser won't ask again); the browser warns once, accept it. Chrome / Edge, Safari 16.4+ or Firefox 130+ are required.

Environment (see `web/docker-compose.yml` and `web/.env.example`):

| Variable | Default | Meaning |
|---|---|---|
| `AIRHAND_SERVER` | (required) | Relay address `host:port` |
| `AIRHAND_SERVER_FP` | empty | Relay certificate fingerprint. Empty means the relay's certificate is remembered on the first connection (`data/server-trust.txt`) and a different one is refused later; delete that file if the relay really gets a new certificate |
| `AIRHAND_NAME` | `Airhand Web` | Name shown on the host as “who is controlling” |
| `AIRHAND_WEB_IDENTITY` | `controller-key.txt` next to `AIRHAND_WEB_CERT` (`/data/controller-key.txt` in the image) | The web console's end-to-end identity (one private key shared by all tabs, mode 600). Hosts paired with the web console recognise it by this key; without a place to save it, it lives in memory only and every restart needs pairing again. End-to-end encryption ends at the web console, so its machine can see the picture (see [Security](security.md#end-to-end-encryption)) |
| `AIRHAND_WEB_PASSWORD` | empty | Access password: the browser shows a login box, any username. Empty means anyone who can reach the port can use the console (connecting to a computer still needs its control code) |
| `AIRHAND_WEB_PORT` | `8080` | Host port mapped to the container |
| `AIRHAND_WEB_CERT_NAMES` | `localhost` | Names / IPs written into a generated HTTPS certificate (comma separated). Put the address you actually use here if you want to import the certificate into the system trust store |
| `AIRHAND_WEB_CERT` / `AIRHAND_WEB_KEY` | `/data/cert.pem` / `/data/key.pem` | HTTPS certificate paths. Drop your own into `web/data/` (or change these paths) |
| `RUST_LOG` | `info` | Log level |

- Every browser tab is its own console with its own connection to the relay; closing or reloading a tab disconnects its sessions (the browser asks first when there are any). The connection to the web service reconnects by itself and the sessions then resume
- Frames are confirmed only once the browser has them, so the host's in-flight control includes the “service → browser” hop and a slow hop there can't pile up latency. The toolbar's latency is the service↔host round trip plus the browser↔service round trip, so deploy the service close to the browsers (same LAN, for example)
- The clipboard is the browser's: text copied on the remote goes into the local clipboard (while the page is in front); text copied locally is sent when you return to the page, which needs clipboard read permission (Chrome / Edge ask once). Firefox doesn't allow pages to read the clipboard automatically, so it only syncs from the remote side
- Outside fullscreen, Ctrl+W, Ctrl+T and friends are taken by the browser; in fullscreen Chrome / Edge lock the keyboard, so Win, Alt+Tab and the rest reach the remote computer
- Behind a reverse proxy (your own domain and certificate): proxy to the container's `http://…:8080`, forward WebSocket (`/ws`) and keep the `Host` header (or set `X-Forwarded-Host`), otherwise the request is rejected as cross-site. A subpath works too
- The image is distroless, about 30 MB, and runs as root (same as the relay: a mounted `web/data/` is writable whatever it belongs to; with rootful Docker its files belong to root on the host). It needs outbound UDP (to the relay) and this one inbound TCP port

## Direct path and ports

| Direction | Port | Who opens it |
|---|---|---|
| Host → relay | outbound UDP 4433 | the host's network |
| Controller → relay | outbound UDP 4433 | the controller's network |
| Relay | inbound UDP 4433 | server security group + firewall |
| Browser → web console | inbound TCP 8080 | the web console machine |
| Host ⇄ controller (direct) | reuses the **same UDP socket** as the relay connection | nothing extra |

After a session starts, the two ends try to punch a direct UDP path; when it works, picture and sound skip the relay (less traffic, lower latency), and when it doesn't the session stays on the relay without breaking. Hole punching uses the same local port as the relay connection, so no extra firewall rule is needed. Symmetric NAT or a firewall that blocks UDP simply means no direct path, which is normal.

## Upgrades, backup and migration

| Task | How |
|---|---|
| Upgrade the relay | `git pull && docker compose up -d --build`, **leave `data/` alone** |
| Back up | Stop the container and back up the whole `data/` (`cert.pem`, `key.pem`, `airhand.db`; to copy while running use `sqlite3 airhand.db ".backup …"` or a filesystem snapshot) |
| Move the relay | Copy the old `data/` along, so the fingerprint stays the same and clients need no change; without it the certificate is new and every client's `server_fp` must change |
| Upgrade clients | Reinstall over the old version on Windows (keeps `config.toml`, so the control code and My devices survive); replace the `.app` on macOS; install the new deb on Linux |
| Forgot the web console password | `AIRHAND_WEB_PASSWORD` is a startup variable: change it and `docker compose up -d` |
| Relay certificate changed | Delete `[server_trust]` from the client's `config.toml`, or update `server_fp` |

## Troubleshooting

| Symptom | Check first |
|---|---|
| Host tray stays grey | Relay address / port; outbound UDP 4433; does `docker compose logs airhand` show the connection |
| Connects but stutters with latency spikes | Host `rmem_max` / `wmem_max`; warnings about capped buffers in the log; relay uplink |
| Client says the certificate changed | Was the relay's `data/` deleted or the machine replaced; if the change is real, update `server_fp` |
| Controller can't reach a host | The control code; hosts ask for the pairing code by default, enter the 6 digits from its tray; 5 wrong attempts pause pairing, change the code to resume |
| Web console won't load | TCP 8080 open; use `https://` from other computers; a reverse proxy must forward `/ws` and keep `Host` |
| Web console loads but can't reach computers | Can the container reach the relay over **outbound UDP** (container networks often only allow TCP) |
