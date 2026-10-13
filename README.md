<div align="center">

<img src="docs/social-preview.png" alt="ProxaLens" width="720" />

# 🔍 ProxaLens

**A free, open-source HTTP/HTTPS debugging proxy for macOS.**
See, tweak, mock and replay your app's network traffic — no subscription needed.

![Platform](https://img.shields.io/badge/platform-macOS-black?logo=apple)
![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-engine-DEA584?logo=rust&logoColor=black)
![React](https://img.shields.io/badge/React-TypeScript-61DAFB?logo=react&logoColor=black)
![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen)

</div>

---

## ✨ Why ProxaLens?

Proxyman-style tooling, but free and hackable. A fast Rust engine does the heavy lifting, a React UI makes it pleasant, and a built-in MCP server lets AI agents inspect your traffic too.

## 🚀 Features

### 📡 Capture
- 🔒 **HTTP / HTTPS MITM proxy** on `127.0.0.1:9090`, starting with the app. HTTP/1.1, HTTP/2 and WebSocket supported.
- 📦 Bodies over 5 MB are stored truncated but forwarded whole.
- 📜 **Per-install root CA**, trusted in your login keychain with one click.
- 🛟 **Crash-safe system proxy:** manual or PAC mode, original settings saved first, and a watchdog restores them if the app is killed.

### 🧐 Inspect
- 🗂️ Three-pane inspector: apps & domains → flows → details.
- 🌳 Bodies as JSON tree, text, hex, image or protobuf; gzip/br/zstd decoded automatically.
- 💬 WebSocket message view, timings, and the app behind each request.
- 🔎 httpmon filter syntax (`m:post s:4xx ct:json re:… app:chrome`) plus method/status chips.

### 🛠️ Debug
- 🔁 Repeat, Edit & Repeat, and Compose.
- ⚖️ Compare two flows side by side.
- 💾 HAR import/export and saved sessions.
- 🐢 Throttling and 📋 Copy as cURL.

### 🎭 Rules & Scripts
- 🗺️ Map Local, Map Remote, Rewrite and Block rules.
- ⏸️ Breakpoints: edit a request or response before it continues.
- 🔐 SSL Proxying include/exclude, plus Block and Allow lists.
- 📝 JavaScript `onRequest` / `onResponse` hooks (httpmon file format) with a Monaco editor.

### 📱 Devices
- Phones and tablets on your network via a setup page with a QR code.
- Certificate install in iOS Simulators.

### 🤖 MCP server
Let agents like Claude Code list, search and replay traffic, add mocks and manage scripts. Off by default — enable it under **Settings → General**.

## 🧑‍💻 Development

**Requirements:** Rust (via `rustup`), Node 20+, Xcode command line tools.

```bash
npm install
npm run tauri dev
```

**Tests**

```bash
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

**Try the proxy without touching system settings:**

```bash
curl -x http://127.0.0.1:9090 \
  --cacert "$HOME/Library/Application Support/dev.naufalzhafran.proxalens/ca/ca-cert.pem" \
  https://httpbin.org/get
```

### 🏗️ Project layout

| Path | What lives there |
| --- | --- |
| `src-tauri/src/engine/` | Tauri-independent proxy engine (store, CA, pipeline, rules, scripts, HAR…) |
| `src-tauri/src/commands.rs` | IPC surface |
| `src-tauri/src/mcp.rs` | MCP server |
| `src/` | React UI |

## 🤝 Contributing

Issues and pull requests are welcome. Please keep `cargo clippy --all-targets` at zero warnings and never touch the real system proxy or keychain in tests.

## 🙏 Credits

Behaviour and test cases are modelled on [kostyay/httpmon](https://github.com/kostyay/httpmon) (MIT). Built on [Tauri](https://tauri.app), [hudsucker](https://github.com/omjadas/hudsucker) and [React](https://react.dev).

<div align="center">

Made with ❤️ by [@naufalzhafran](https://github.com/naufalzhafran)

</div>
