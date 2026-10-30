# ProxaLens

A free, Proxyman-style HTTP/HTTPS debugging proxy for macOS, built with Tauri v2, a Rust engine and a React + TypeScript UI.

- Layout:
  - `src-tauri/src/engine/` is the Tauri-independent proxy engine: store, ca, proxy pipeline, sysproxy and guard, rules, decoding, HAR, throttle, timing (timed upstream connector), procinfo, settings.
  - `src-tauri/src/commands.rs` is the IPC surface.
  - `src/` is the React UI (`api.ts` mirrors the command types).
- Commands: `npm run tauri dev`; `npm test`; `cargo test --manifest-path src-tauri/Cargo.toml`; `cargo clippy --all-targets` (in `src-tauri`). Keep clippy at zero warnings.
- Never touch the real system proxy or keychain in tests. Use `FakeNs` (`engine/sysproxy.rs` tests) and temp dirs.
- Reference implementation: [kostyay/httpmon](https://github.com/kostyay/httpmon) (Go, MIT). Port its behaviour and tests; don't ship its code.
