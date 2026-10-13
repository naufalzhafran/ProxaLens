# Project Proxy

A free, Proxyman-style HTTP/HTTPS debugging proxy for macOS. Built with Tauri v2, a Rust proxy engine ([hudsucker](https://github.com/omjadas/hudsucker)), and React + TypeScript.

Status: early development. See [docs/PLAN.md](docs/PLAN.md) for the roadmap and progress.

## What works today (Phases 0–1)
- An HTTP/HTTPS MITM proxy on `127.0.0.1:9090` that starts with the app. HTTP/1.1 and HTTP/2 work, and WebSocket messages are captured.
- A root CA generated per install. One click trusts it in the System keychain (macOS asks for your password).
- A live, virtualized flow list with a filter, and a detail panel showing headers and bodies. Bodies over 5 MB are stored truncated but forwarded whole.
- A system proxy toggle with crash-safe restore. Your original settings are saved before any change, and a watchdog process puts them back if the app is killed. See "System proxy crash safety" in the plan.

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
curl -x http://127.0.0.1:9090 --cacert "$HOME/Library/Application Support/dev.naufalzhafran.projectproxy/ca/ca-cert.pem" https://httpbin.org/get
```

## Credits
The behaviour and test cases are modelled on [kostyay/httpmon](https://github.com/kostyay/httpmon) (MIT).
