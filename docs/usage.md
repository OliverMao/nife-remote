# Usage guide

[← Back to README](../README.md) · [简体中文](usage.zh-CN.md)

## Host and controller at the same time

Double-click to run (on Windows install it first, see [Windows installation](#windows-installation)): the console window opens, and **the top of the home page shows this computer's control code and pairing code**. The pairing code has a **Change** button: pick your own 6 digits and it takes effect immediately. Others use that code to connect here, and you can connect out from the same window; the two halves don't affect each other. Closing the window only disconnects your own sessions, this computer stays reachable, and **Open console** in the tray brings the window back.

The tray icon sits in the bottom-right corner. Grey means not connected to the server, coloured means connected. The right-click menu contains:

- This computer's control code (also "Copy control code") and the server status
- The **pairing code**, "Copy pairing code" and **Change pairing code**: a controller that hasn't paired with this computer needs it, besides the control code, the first time it connects (see "Pairing" below)
- Who is controlling this computer right now (with the **safety code** after each connection), and **Disconnect all controllers**
- **Change control code**: the old code stops working immediately and connected controllers are disconnected; the pairing code changes too and all paired controllers are forgotten
- **Forget paired controllers**: they'll need the pairing code again
- **Open console**, **Start with Windows** / Language / Quit

After a network drop the host reconnects automatically with exponential backoff and **gets its old control code back**, even after a server restart. The code is **fixed**: it's saved in `config.toml`, and the server keeps it for this computer however long it stays offline (codes unused for 180 days are released), so it stays the same across restarts and upgrades until you change it.

**Pairing**: the relay server assigns control codes, so it knows all of them; the control code alone therefore doesn't get anyone in (otherwise the server could connect by itself). The first time a controller connects to this computer it has to enter the 6-digit **pairing code** from the tray menu (or the console's home page); the pairing code never goes through the server. After that this computer remembers the controller and the control code is enough. After 5 wrong pairing codes in a row pairing is paused (the tray says so) until you pick a new pairing code. If you don't want pairing codes (e.g. you only use a relay server you trust), set `access = "open"` in `config.toml`. See [Security → End-to-end encryption](security.md#end-to-end-encryption).

## Windows installation

On Windows Nife Remote is installed with its installer (into `C:\Program Files\Nife Remote` for all users; needs administrator rights). The host runs as the system service **Nife Remote**, as SYSTEM in the session of whoever is at the screen, so it keeps working on the lock screen, the sign-in screen and UAC prompts, and follows when users are switched. The tray icon is an ordinary program that shows the service's status and controls it:

- Opening Nife Remote starts the service; **Quit** in the tray stops it (as before, the computer can't be controlled after quitting); when the service stops (or is uninstalled) the tray icons close too
- **Start with Windows** (tray menu, on by default): the service starts at boot, so the computer can be controlled before anyone signs in, and the tray icon appears after signing in
- The configuration file `config.toml` is in the install directory (created on installation, see [Configuration file](#configuration-file)) and maintained by the service; edit it as administrator
- **Uninstalling** (Settings → Apps) stops and removes the service, the startup entry, `config.toml` and the program; nothing is left behind (see [Uninstalling](#uninstalling)). Installing a new version over an old one keeps `config.toml`, and with it the control code and My devices, even if you choose to uninstall the old version first
- A bare `nife-remote.exe` doesn't run without being installed: release builds ask you to run the installer. Debug builds (`npm run tauri dev`) run the host inside the tray process, with `config.toml` next to the program (`client/src-tauri/target/debug/`)

## Configuration file

Everything Nife Remote saves is in one `config.toml` (with comments). The menu in the console's top-left corner also shows where it is:

| Location | Notes |
|---|---|
| `C:\Program Files\Nife Remote\config.toml` | The install directory (next to the program for a debug build) |

| Key | Description |
|---|---|
| `server` | Relay server `host:port`; without it, the address built into the program |
| `server_fp` | Relay certificate fingerprint; `""` means no fixed fingerprint: the server's certificate is remembered on the first connection (`[server_trust]`) and a different one is refused later; without it, the built-in fingerprint |
| `[server_trust]` | Written by Nife Remote: the relay's certificate fingerprint remembered on the first connection (when no fingerprint is set). Delete it if the relay really gets a new certificate |
| `access` | Whether controllers that haven't paired may connect: `"pairing"` (default) needs the pairing code from the tray; `"open"` lets anyone with the control code in (which means trusting the relay server) |
| `autostart` | Start with Windows (also in the tray menu) |; System Settings is what counts, so editing only this line has no effect. Without it, Open at login hasn't been turned on by default yet (see above) |
| `[code]` | This computer's control code, and the secret that gets it back. Delete `code` and `secret` to get a new code |
| `[host]` | Maintained by Nife Remote: this computer's end-to-end identity as a host (`key`), the pairing code and whether pairing is paused, all stored encrypted. Deleting `key` creates a new identity: controllers that connected before will warn that this computer's identity changed |
| `[[paired]]` | Maintained by Nife Remote: paired controllers (public key, name, when they paired); they connect without the pairing code. Delete an entry to forget that controller |
| `[controller]` | Maintained by Nife Remote: the console's end-to-end identity (stored encrypted, one per account on Windows). Paired computers recognize the console by it; delete it and you'll have to pair again |
| `[[device]]` | The console's **My devices**: computers you've connected to (control code, their name, the name you gave them, last quality settings, last connected, their public key `host_key`). Saved automatically on connect, up to 50. To add one by hand, write the 6 digits as `code` |

- Restart Nife Remote after changing settings (Quit, then open it again). Nife Remote keeps your edits and comments when it saves the code or devices
- **Deleted by accident** (the file or the whole directory): it's recreated with the default settings at startup, and within a few seconds while running. This computer's control code, the autostart setting, the end-to-end identities, the paired controllers and My devices carry over from before the deletion, so nothing is lost; other settings go back to their defaults
- A control code is enough to control its computer, so treat it like a key. The file is in the install directory, readable by every account on the computer: this computer's secret is encrypted with DPAPI for the SYSTEM account, and the codes in My devices with the Windows account that saved them, so other accounts and other computers can't read them (each account on the same computer only sees its own devices)
- The uninstaller removes it, see [Uninstalling](#uninstalling)

## Uninstalling

After these steps nothing of Nife Remote is left on the computer.

### Windows

Settings → Apps → Nife Remote → Uninstall. The uninstaller stops and removes the Nife Remote service and the startup entry, and deletes the install directory (including `config.toml`), the registry keys the installer wrote, the Windows Firewall rules added for `nife-remote.exe`, and the console's temporary directories. Nothing else to do by hand.

(Choosing "uninstall the old version first" during an upgrade isn't an uninstall: `config.toml` is kept, see [Windows installation](#windows-installation).)

## Controller

The console window is the controller. It opens at startup; **closing it only hides the window** (no confirmation; the controller and the host keep running and live sessions are not dropped) and **Open console** in the tray brings it back. To quit the program, use **Quit** in the tray icon's right-click menu.

- It works like a browser: each computer gets a **tab** that you can drag to reorder and close (= disconnect) with × or a middle click. Click **+** for a new connection, enter the 6-digit control code (pasting `123 456` works), and once connected that tab becomes the computer. With two or more connections a **Multi-screen overview** tab appears on the left
- **First connection to a computer**: when it asks for a pairing code (desktop hosts do by default), the new-connection page shows a "Pairing code" field: enter the 6-digit pairing code from its tray menu and connect again (or paste all 12 digits at once: control code + pairing code). Once paired, it remembers this console
- **End-to-end encryption**: sessions are encrypted end to end, so the relay server can't see the screen or keystrokes. The console remembers a computer's identity the first time it connects; if the identity changes later (reinstalled, or someone impersonating it), the console warns and doesn't reconnect by itself. Once you've confirmed it was reinstalled, enter its current pairing code from its tray to pair again. Hover the control code in the toolbar to see the **safety code**: if the remote computer's tray shows the same one, nobody is in between
- **My devices**: computers you connect to are saved (in `config.toml`); next time, one click on the new-connection page connects with the quality settings you used last. Rename or remove them (with undo), and search when there are many. If one is offline, there's no error: its tab opens and waits, and connects as soon as it's back (using the saved reconnect token, which doesn't count as code guessing)
- The window has no system title bar: drag an empty part of the tab bar to move it, double-click to maximize. The gear on the right of the tab bar is the menu: this computer's name, language, config file, shortcuts. Closing the window (including from the taskbar or Alt+F4) only hides it, it does not quit the program
- Shortcuts (when not controlling a computer): Ctrl+T new connection, Ctrl+W close tab, Ctrl+Tab / Ctrl+Shift+Tab switch tabs, Ctrl+1–9, Ctrl+Shift+T reopen the last closed connection. While the remote screen has focus, every key goes to the remote computer
- Toolbar (like an address bar): remote name, control code, frame rate / bitrate / latency, encoder; monitor switch, **Quality**, shortcuts, clipboard (two-way sync / type as keystrokes), sound (mute / A/V sync), microphone (only when the remote computer can take it, see [Microphone](#microphone) below), actual size (1:1), fullscreen, disconnect
- In fullscreen the tab bar is hidden and the toolbar slides in when the mouse touches the top edge; click the remote name to switch to another connection without leaving fullscreen. The console also tries to lock the keyboard, so system shortcuts such as the Win key and Alt+Tab go to the remote computer
- **Turning the camera in games**: when the remote computer hides its cursor (holding the right button to turn the camera in World of Warcraft, first-person games), the mouse switches to relative movement and the local pointer is locked to the picture, so it never stops at the edge of the screen. It's released when the remote cursor comes back (if a mouse button is held, once you let go: some games make the cursor flicker while the camera turns), or with **Ctrl+Alt+Shift+M** or by holding Esc (about a second; in fullscreen the keyboard is locked, so a short Esc still goes to the remote computer). If the game doesn't hide the cursor, lock the mouse yourself with the mouse button in the toolbar or **Ctrl+Alt+Shift+M**; release it the same way, with Ctrl+Alt+Shift+M or by holding Esc
- Network hiccups reconnect automatically. If the user at the host disconnects you, the console says so and doesn't reconnect
- If the remote computer goes offline (shut down, asleep, network gone), the console shows "Waiting for the remote computer to come back" and reconnects as soon as it's back with its old code. It doesn't keep retrying in the meantime, and the server doesn't mistake the reconnects for code guessing

**Quality panel**: presets from Data saver (720p / 24 fps / 1.2 Mbps) through Balanced (1080p / 30 fps / 4 Mbps), Smooth (1080p / 60 fps / 8 Mbps) and HD (1440p / 60 fps / 16 Mbps) to Original (native resolution / 60 fps / 30 Mbps), or set the resolution cap, frame rate (15–120), bitrate cap (0.5–50 Mbps), codec and adaptive bitrate yourself. The console only offers codecs this computer can hardware-decode; if the host has no matching hardware encoder it falls back automatically, and the toolbar shows the encoder actually in use.

**Scaling**: how the picture is shrunk when the remote screen is larger than the requested resolution (a 4K or Retina screen down to 1080p, say). **Low latency** (default) scales on the remote GPU, frames never touch its CPU, so latency and its CPU usage are much lower; **Sharp text** scales on the remote CPU with a sharper filter, so small text stays clearer when shrinking a lot, at the cost of roughly 10–30 ms more per frame and higher CPU usage on the remote computer. Without scaling the two are the same. If the remote GPU can't do it, the CPU is used automatically.

Besides the resolution cap, the picture is never larger than what the window actually shows: when the window is smaller than the remote screen, the host scales straight to the window size and the console shows it 1:1 instead of scaling it a second time (sharper text, less bandwidth). While you drag the window border, the resolution changes once you stop. In actual size (1:1) mode the window doesn't limit the resolution.

**Enhance (this device)**: Off / Low / Medium / High, off by default. When the picture is smaller than the window and has to be enlarged (a 1080p preset in full screen on a 2K / 4K screen, say), it's upscaled on this computer's GPU with a sharper filter (Catmull-Rom) and text edges are sharpened a little, on brightness only; at 1:1 it's only sharpened. No extra bandwidth and practically no extra latency. Strong settings also make compression artifacts (blocks, ragged text edges) more visible, so pick Low at low bitrates. It only applies to the connection you're working in (thumbnails in the overview aren't processed), applies to all connections, and is remembered on this computer. Without WebGL2 the browser's own high-quality scaling is used instead (no sharpening). To actually get more detail, raise the resolution cap and bitrate: enhancement can only display what's received better.

**Local cursor (this device)**: on by default. A Windows host draws its cursor straight into the picture while a window is being dragged or resized. On: this computer's cursor stays visible and follows your hand with no delay, next to the (slightly slower) one in the picture. Off: only the one in the picture is shown. Either way the mouse keeps working by position and isn't locked. Applies to all connections and is remembered on this computer.

### Microphone

Send this computer's microphone to the remote computer: programs there (meetings, voice typing, voice messages) hear you when they record from its virtual microphone. Off by default and turned on per connection (the microphone button in the toolbar → **Send this computer's microphone**); it's only sent while you're viewing that connection. The browser asks for microphone access the first time (the desktop console doesn't).

- **Windows hosts** use Steam Remote Play's virtual sound card, **Steam Streaming Microphone** (Valve's driver, signed by Microsoft). It's there once Steam is installed on the remote computer and Steam Remote Play has been used once; Nife Remote neither ships nor installs it. Without it, the microphone menu says so
- In programs on the remote computer, choose "Microphone (Steam Streaming Microphone)" as the recording device, or make it the default under Sound settings → Input there, so programs that use the default microphone (such as Windows voice typing, Win+H) hear you directly
- Only one controller at a time: another controller sees "Another controller is using the remote microphone" and takes over when that one stops

## Language

The console and tray menu are available in **English** and **Simplified Chinese**. By default the client follows the system display language: Chinese if it is Chinese, English otherwise. Switch any time from the tray menu (Language / 语言) or the 中文 / EN switch in the console menu (the icon at the top-left). The choice isn't saved: the next start follows the system language again.

Notices and errors from the other computer or the server are sent as message types, not text, so they always appear in **your** language. Logs (client and server) are always in English.
