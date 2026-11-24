# Usage guide

[← Back to README](../README.md) · [简体中文](usage.zh-CN.md)

## Host and controller at the same time

Double-click to run (on Windows install it first, see [Windows installation](#windows-installation)): the console window opens, and **the top of the home page shows this computer's control code and pairing code**. The pairing code has a **Change** button: pick your own 6 digits and it takes effect immediately. Others use that code to connect here, and you can connect out from the same window; the two halves don't affect each other. Closing the window only hides it (remote windows are separate and keep running), this computer stays reachable, and **Show window** in the tray (or a left click on the icon) brings the window back.

The tray icon sits in the bottom-right corner. Grey means not connected to the server, coloured means connected (hover it to see the status and this computer's control code). The **right-click menu has just two items**:

- **Show window** (a left click on the icon does the same)
- **Quit**: really exits; on an installed Windows build the host service stops too, so this computer can't be controlled anymore

The control code, the pairing code, "Disconnect all controllers" and the language all live in the console: codes are on the **Remote assistance** page, the controllers on the **Being controlled** page, language and quality in the gear menu at the top right.

After a network drop the host reconnects automatically with exponential backoff and **gets its old control code back**, even after a server restart. The code is **fixed**: it's saved in `config.toml`, and the server keeps it for this computer however long it stays offline (codes unused for 180 days are released), so it stays the same across restarts and upgrades until you change it.

**Pairing**: the relay server assigns control codes, so it knows all of them; the control code alone therefore doesn't get anyone in (otherwise the server could connect by itself). The first time a controller connects to this computer it has to enter the 6-digit **pairing code** from the other computer's Remote assistance page; the pairing code never goes through the server. After that this computer remembers the controller and the control code is enough. After 5 wrong pairing codes in a row pairing is paused until you pick a new pairing code on that page. If you don't want pairing codes (e.g. you only use a relay server you trust), set `access = "open"` in `config.toml`. See [Security → End-to-end encryption](security.md#end-to-end-encryption).

## Windows installation

On Windows Nife Remote is installed with its installer (into `C:\Program Files\Nife Remote` for all users; needs administrator rights). The host runs as the system service **Nife Remote**, as SYSTEM in the session of whoever is at the screen, so it keeps working on the lock screen, the sign-in screen and UAC prompts, and follows when users are switched. The tray icon is an ordinary program; its right-click menu has just Show window and Quit:

- Opening Nife Remote starts the service; **Quit** in the tray stops it (as before, the computer can't be controlled after quitting); when the service stops (or is uninstalled) the tray icons close too
- **Start with Windows** (on by default at install time, see `autostart` in `config.toml`): the service starts at boot, so the computer can be controlled before anyone signs in, and the tray icon appears after signing in
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
| `[relay]` | Written by Nife Remote: the relay address and fingerprint assigned by the account service. Editing it has no effect — it is replaced the next time the account service answers |
| `access` | Whether controllers that haven't paired may connect: `"pairing"` (default) needs the pairing code from the tray; `"open"` lets anyone with the control code in (which means trusting the relay server) |
| `autostart` | Start with Windows (no UI switch; restart Nife Remote after editing) |; System Settings is what counts, so editing only this line has no effect. Without it, Open at login hasn't been turned on by default yet (see above) |
| `[code]` | This computer's control code, and the secret that gets it back. Delete `code` and `secret` to get a new code |
| `[host]` | Maintained by Nife Remote: this computer's end-to-end identity as a host (`key`), the pairing code and whether pairing is paused, all stored encrypted. Deleting `key` creates a new identity: controllers that connected before will warn that this computer's identity changed |
| `[[paired]]` | Maintained by Nife Remote: paired controllers (public key, name, when they paired); they connect without the pairing code. Delete an entry to forget that controller |
| `[controller]` | Maintained by Nife Remote: the console's end-to-end identity (stored encrypted, one per account on Windows). Paired computers recognize the console by it; delete it and you'll have to pair again |
| `[[device]]` | This computer's connection records (control code, their name, the name you gave them, last quality settings, last connected, their public key `host_key`). Saved automatically on connect, up to 50. The **My devices** list itself lives in the account service (the devices signed in to the account); these records only supply names, quality and reconnect tokens for it |

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

The console window is the controller. It opens at startup; **closing it only hides the window** (no confirmation; the controller and the host keep running and live sessions are not dropped) and **Show window** in the tray (or a left click) brings it back. To quit the program, use **Quit** in the tray icon's right-click menu.

- **One window per computer**: once connected, the remote screen lives in **its own window**; the console window only opens it and brings it back to the front, it never shows the remote picture itself. Clicking the same device again just brings its window forward instead of connecting twice, and closing that window disconnects that one session. **Sessions** in the left bar lists the windows that are open: click one to bring it forward
- **First connection to a computer**: when it asks for a pairing code (desktop hosts do by default), the remote window shows a "Pairing code" field: enter the 6-digit pairing code from its Remote assistance page and connect again. Once paired, it remembers this console
- **End-to-end encryption**: sessions are encrypted end to end, so the relay server can't see the screen or keystrokes. The console remembers a computer's identity the first time it connects; if the identity changes later (reinstalled, or someone impersonating it), the console warns and doesn't reconnect by itself. Once you've confirmed it was reinstalled, enter its current pairing code from its tray to pair again. Hover the control code in the toolbar to see the **safety code**: if the remote computer's tray shows the same one, nobody is in between
- **My devices**: the devices **signed in to the same account** (same-account devices trust each other, no pairing code). Each device connects to the account service when it starts; the service keeps the device list with names and online state and pushes it to all of them, so the list shows **device names** rather than control codes (codes are not shown anywhere in the UI — see "Remote assistance" below). This computer is in the list too, marked "This computer". One click connects with the quality settings you used last, in that computer's own window. Rename or remove them (removing takes the entry out of the account; it shows up again when that device signs in next), and search when there are many. If one is offline, there's no error: its window stays open and waits, and connects as soon as it's back (using the saved reconnect token, which doesn't count as code guessing)
- The console window has no system title bar: drag an empty part of the title bar to move it, double-click to maximize. The gear on the right is the menu: this computer's name, language, config file. Closing the window (including from the taskbar or Alt+F4) only hides it, it does not quit the program. Remote windows have no system title bar either: the top bar is their title bar (icon, remote name, control code, display tabs, window buttons); drag an empty spot to move the window, double-click to maximize, and the taskbar shows the remote computer's name
- Title bar (like a browser's tab strip): icon, remote name, control code, frame rate / bitrate / latency / encoder, and **display tabs** (only when the remote has several displays): click one to switch, or **drag it out of the window and release** to open that display in its own window (two windows, two connections, both visible at once; a display belongs to one window, so it disappears from this window's tabs). On the right is the **Control center** and nothing else — every control lives in it: quality, sound (off by default, turn it on when you need it) (mute / A/V sync), clipboard sync / send clipboard, this computer's microphone, lock the mouse, picture size (fit / actual size), send shortcuts, fullscreen, disconnect
- In fullscreen the toolbar is hidden and slides in when the mouse touches the top edge. The console also tries to lock the keyboard, so system shortcuts such as the Win key and Alt+Tab go to the remote computer
- **Turning the camera in games**: when the remote computer hides its cursor (holding the right button to turn the camera in World of Warcraft, first-person games), the mouse switches to relative movement and the local pointer is locked to the picture, so it never stops at the edge of the screen. It's released when the remote cursor comes back (if a mouse button is held, once you let go: some games make the cursor flicker while the camera turns), or with **Ctrl+Alt+Shift+M** or by holding Esc (about a second; in fullscreen the keyboard is locked, so a short Esc still goes to the remote computer). If the game doesn't hide the cursor, lock the mouse yourself with the mouse button in the toolbar or **Ctrl+Alt+Shift+M**; release it the same way, with Ctrl+Alt+Shift+M or by holding Esc
- Network hiccups reconnect automatically. If the user at the host disconnects you, the remote window says so and doesn't reconnect
- If the remote computer goes offline (shut down, asleep, network gone), the remote window shows "Waiting for the remote computer to come back" and reconnects as soon as it's back with its old code. It doesn't keep retrying in the meantime, and the server doesn't mistake the reconnects for code guessing
- **If the remote computer is locked or its monitor is powered off**, the picture stays black: Windows desktop duplication has nothing to capture while the session is locked or the display has no signal. If no frame arrives within 4 seconds of connecting, the remote window says "The other computer isn't sending any picture"; the picture comes back on its own once the remote computer is unlocked and its display is on, no reconnect needed

**Quality panel**: presets from Data saver (720p / 24 fps / 1.2 Mbps) through Balanced (1080p / 30 fps / 4 Mbps), Smooth (1080p / 60 fps / 8 Mbps) and HD (1440p / 60 fps / 16 Mbps) to Original (native resolution / 60 fps / 30 Mbps), or set the resolution cap, frame rate (15–120), bitrate cap (0.5–50 Mbps), codec and adaptive bitrate yourself. The console only offers codecs this computer can hardware-decode; if the host has no matching hardware encoder it falls back automatically, and the toolbar shows the encoder actually in use. **Automatic** selection only picks between H.265 and H.264 (the one with better compression first); AV1 has to be picked by hand and is marked experimental, because some computers' WebView can't decode it (the config check says it's supported and the picture is black). If AV1 is picked and no frame shows up, it switches back to H.265 / H.264 after 4 seconds and says so.

**Scaling**: default **Sharp text**, switch it to low latency if you prefer. How the picture is shrunk when the remote screen is larger than the requested resolution (a 4K or Retina screen down to 1080p, say). **Low latency** scales on the remote GPU, frames never touch its CPU, so latency and its CPU usage are much lower; **Sharp text** scales on the remote CPU with a sharper filter, so small text stays clearer when shrinking a lot, at the cost of roughly 10–30 ms more per frame and higher CPU usage on the remote computer. Without scaling the two are the same. If the remote GPU can't do it, the CPU is used automatically.

Besides the resolution cap, the picture is never larger than what the window actually shows: when the window is smaller than the remote screen, the host scales straight to the window size and the console shows it 1:1 instead of scaling it a second time (sharper text, less bandwidth). While you drag the window border, the resolution changes once you stop. In actual size (1:1) mode the window doesn't limit the resolution.

**Enhance (this device)**: Off / Low / Medium / High, **Low** by default. When the picture is smaller than the window and has to be enlarged (a 1080p preset in full screen on a 2K / 4K screen, say), it's upscaled on this computer's GPU with a sharper filter (Catmull-Rom) and text edges are sharpened a little, on brightness only; at 1:1 it's only sharpened. No extra bandwidth and practically no extra latency. Strong settings also make compression artifacts (blocks, ragged text edges) more visible, so pick Low at low bitrates. It applies to all connections, and is remembered on this computer. Without WebGL2 the browser's own high-quality scaling is used instead (no sharpening). To actually get more detail, raise the resolution cap and bitrate: enhancement can only display what's received better.

**Local cursor (this device)**: on by default. A Windows host draws its cursor straight into the picture while a window is being dragged or resized. On: this computer's cursor stays visible and follows your hand with no delay, next to the (slightly slower) one in the picture. Off: only the one in the picture is shown. Either way the mouse keeps working by position and isn't locked. Applies to all connections and is remembered on this computer.

### File transfer

Control center → **File transfer**: opens a window with this computer's folder on the left, the remote computer's folder on the right, and two **Send** buttons in between.

- Click a file name to select it, click a folder to enter it; `↑` up one level, `⌂` default folder, `⟳` refresh
- **Send →** sends the file selected on the left into the remote folder; **← Send** pulls the file selected on the right into the local folder
- If the name already exists you get asked: **Skip / Replace / Keep both** (`name (1).ext`)
- The list at the bottom shows status (with a percentage while sending), size, source and target path; finished entries can be cleared
- Single files only (no folders, no resume); transfers ride the session's bulk lane (same one the clipboard uses) and are not counted against the quality bitrate cap
- With an older remote version that doesn't advertise the capability, the control center doesn't show File transfer

### Microphone

Send this computer's microphone to the remote computer: programs there (meetings, voice typing, voice messages) hear you when they record from its virtual microphone. Off by default and turned on per connection (the microphone button in the toolbar → **Send this computer's microphone**); it's only sent while you're viewing that connection. The browser asks for microphone access the first time (the desktop console doesn't).

- **Windows hosts** use Steam Remote Play's virtual sound card, **Steam Streaming Microphone** (Valve's driver, signed by Microsoft). It's there once Steam is installed on the remote computer and Steam Remote Play has been used once; Nife Remote neither ships nor installs it. Without it, the microphone menu says so
- In programs on the remote computer, choose "Microphone (Steam Streaming Microphone)" as the recording device, or make it the default under Sound settings → Input there, so programs that use the default microphone (such as Windows voice typing, Win+H) hear you directly
- Only one controller at a time: another controller sees "Another controller is using the remote microphone" and takes over when that one stops

## Language

The console is available in **English** and **Simplified Chinese**. By default the client follows the system display language: Chinese if it is Chinese, English otherwise. Switch any time from the 中文 / EN switch in the console menu (the gear at the top-right). The choice isn't saved: the next start follows the system language again.

Notices and errors from the other computer or the server are sent as message types, not text, so they always appear in **your** language. Logs (client and server) are always in English.
