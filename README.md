# LocalFTP

Zero-config local file transfer desktop app built with Tauri v2, Rust, React, and Tailwind CSS.

## Features

- Automatic peer discovery over Wi-Fi / Hotspot via mDNS (`_localftp._tcp.local.`)
- Direct file streaming with Accept / Decline confirmation
- Mobile QR Bridge for sending and receiving files from iOS / Android browsers
- macOS translucent vibrancy window with Dark / Light frost modes and 5 accent themes

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build -- --bundles app
```
