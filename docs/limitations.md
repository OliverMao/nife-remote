# Known limitations

[← Back to README](../README.md) · [简体中文](limitations.zh-CN.md)

## General

- The clipboard syncs text only, not files
- Audio is captured after the host's volume control, so a muted host sends silence
- The controller's microphone only reaches Windows hosts that have Steam Streaming Microphone (Steam installed and Steam Remote Play used once): Nife Remote has no virtual sound card driver of its own
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

## Web console

- Reloading or closing the tab disconnects its sessions; My devices isn't saved and is cleared when the page reloads
- In Firefox, text copied locally isn't synced to the remote side automatically
- Outside fullscreen some shortcuts (Ctrl+W…) can't reach the remote computer
