# ProxaLens

A free, Proxyman-style HTTP/HTTPS debugging proxy for macOS. Built with Tauri v2, a Rust proxy engine ([hudsucker](https://github.com/omjadas/hudsucker)), and React + TypeScript.

## Features
- **Capture:** an HTTP/HTTPS MITM proxy on `127.0.0.1:9090` that starts with the app. HTTP/1.1, HTTP/2 and WebSocket are supported. Bodies over 5 MB are stored truncated but forwarded whole.
- **Certificate:** a root CA generated per install. One click trusts it in your login keychain (macOS asks for your password).
- **Crash-safe system proxy:** "Proxy On" switches every network service, using manual HTTP/HTTPS settings or an automatic PAC URL. Your original settings are saved first, and a watchdog process puts them back if the app is killed.
- **Inspector:** three panes (apps and domains, flows, inspector). Bodies can be shown as a JSON tree, text, hex, image or protobuf. Compressed bodies (gzip/br/zstd) are decoded. There's also a WebSocket message view, timings, and the app that made each request.
- **Filters:** httpmon's filter syntax (`m:post s:4xx ct:json re:… app:chrome`), plus method and status chips.
- **Debug tools:**
  - Repeat, Edit and Repeat, and Compose.
  - Compare two flows.
  - HAR import/export, and saved sessions.
  - Throttling.
  - Copy as cURL.
- **Rules:**
  - Map Local, Map Remote, Rewrite and Block rules.
  - Breakpoints that let you edit a request or response before it continues.
  - SSL Proxying include/exclude, and Block and Allow lists.
- **Scripts:** JavaScript `onRequest`/`onResponse` hooks in httpmon's file format, with an in-app editor (Monaco).
- **Devices:** phones and tablets on your network (setup page with a QR code), and certificate install in iOS Simulators.
- **MCP server:** lets agents such as Claude Code list, search and replay traffic, add mocks and manage scripts. It's off by default; turn it on under Settings → General.

## Not yet supported
- A privileged helper for password-free proxy switching (it needs a signed app).
- Windows.

## Development
Requirements: Rust (via `rustup`), Node 20+, and Xcode command line tools.

```bash
npm install
npm run tauri dev
```

Tests:

```bash
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

To try the proxy without changing system settings, use curl:

```bash
curl -x http://127.0.0.1:9090 --cacert "$HOME/Library/Application Support/dev.naufalzhafran.proxalens/ca/ca-cert.pem" https://httpbin.org/get
```

## Credits
The behaviour and test cases are modelled on [kostyay/httpmon](https://github.com/kostyay/httpmon) (MIT).
