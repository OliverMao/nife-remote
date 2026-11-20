# Known limitations

[← Back to README](../README.md) · [简体中文](limitations.zh-CN.md)

## General

- The clipboard syncs text only, not files
- Audio is captured after the host's volume control, so a muted host sends silence
- The controller's microphone only reaches Windows hosts that have Steam Streaming Microphone (Steam installed and Steam Remote Play used once): Airhand has no virtual sound card driver of its own. macOS and Linux hosts can't take it yet
- Sessions are end-to-end encrypted, so the relay can't see their content, but it still knows who connected to whom, when, and how much traffic flowed (see [Security](security.md#end-to-end-encryption))
- The web console is where end-to-end encryption ends: the machine running it can see the picture
- On the first connection to a computer its identity is whatever was seen then (like SSH): only the pairing code (or comparing the safety code) proves nobody is impersonating it
- The direct path relies on hole punching only: when both ends are behind symmetric NATs (common with some carriers' carrier-grade NAT and on corporate networks) and don't both have public IPv6, or a firewall blocks UDP, it doesn't work and the session stays on the relay; there's no UPnP / NAT-PMP port mapping. When an established direct path suddenly breaks (for example the host switches networks), the video stalls for about a second (until the host notices, moves back to the relay and encodes a keyframe) before the direct path is tried again
- While the direct path is up, the control lane is sent on both paths (the relay always has a complete copy) and the bulk lane (clipboard, cursor images) only over the relay: the relay carries much less traffic without video and audio, but not none

## Windows

- The host only runs in the console session (the physical screen): Remote Desktop (RDP) sessions can't be controlled
- `Ctrl+Alt+Del` can't be sent yet (it needs `SendSAS`)
- Debug builds run the host in the tray process, without SYSTEM rights, so they can't capture or control UAC prompts and the lock screen (the controller shows a capture error meanwhile)
- The controller's microphone is written into Steam Streaming Microphone in exclusive mode when its two devices ("Speakers" and "Microphone" in the sound settings) have different default formats (the usual case): while a controller's microphone is on, Steam Remote Play can't use it and vice versa, and it can't be opened if "Allow applications to take exclusive control of this device" is turned off (giving both devices the same default format avoids exclusive mode). It isn't made the default recording device automatically: choose it in the program, or set it in the sound settings yourself

## macOS

- The host needs the Screen & System Audio Recording and Accessibility permissions, which only the local user can grant in System Settings; since macOS 15 the system also asks again from time to time whether to keep allowing screen recording
- The login window and the lock screen can't be captured or controlled
- The host doesn't offer AV1 (current Apple chips can only decode it in hardware), so AV1 shows as unavailable in the console and HEVC is used instead
- The controller (WKWebView) can't lock the keyboard: in fullscreen, Command+Tab, Command+Space and the like are still handled locally, so use "Send shortcut" in the toolbar
- WKWebView on older macOS versions may lack Opus decoding, in which case the console says remote audio can't be played

## Linux

- The host only supports **X11** desktops. Under Wayland only X11 (XWayland) applications can be seen and controlled; native Wayland windows are black and the controller suggests logging in with an X11 (Xorg) session. Full Wayland support needs xdg-desktop-portal screen sharing (the local user has to confirm each time) and isn't implemented
- The host only has OpenH264 software encoding (no VA-API / NVENC yet), so the controller uses H.264
- The lock screen can't be controlled (the screensaver grabs the keyboard and mouse)
- Decoding on the controller relies on WebKitGTK + GStreamer: H.264 needs `gstreamer1.0-plugins-bad` (parsing) and `gstreamer1.0-libav` or openh264 (decoding); AV1 / HEVC depend on the installed GStreamer decoders. If a format turns out not to produce a picture, the controller switches to the next available format by priority

## Web console

- Reloading or closing the tab disconnects its sessions; My devices isn't saved and is cleared when the page reloads
- In Firefox, text copied locally isn't synced to the remote side automatically
- Outside fullscreen some shortcuts (Ctrl+W…) can't reach the remote computer
