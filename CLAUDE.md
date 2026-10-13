# project-proxy

A free, Proxyman-style HTTP/HTTPS debugging proxy for macOS, built with Tauri v2, a Rust engine and a React + TypeScript UI.

- **Read `docs/PLAN.md` first.** It holds the assessment, the architecture, the phased roadmap and a status checklist.
- Tick the checklist in `docs/PLAN.md` when you finish a phase or a notable step, and commit it with the work.
- Layout: `src-tauri/src/engine/` is the Tauri-independent proxy engine (store, ca, proxy, sysproxy, guard). `src-tauri/src/commands.rs` is the IPC surface, and `src/` is the React UI (`api.ts` mirrors the command types).
- Commands: `npm run tauri dev`; `npm test`; `cargo test --manifest-path src-tauri/Cargo.toml`; `cargo clippy --all-targets` (in `src-tauri`). Keep clippy at zero warnings.
- Never touch the real system proxy or keychain in tests. Use `FakeNs` (`engine/sysproxy.rs` tests) and temp dirs.
- Reference implementation: [kostyay/httpmon](https://github.com/kostyay/httpmon) (Go, MIT). Port its behaviour and tests; don't ship its code.
