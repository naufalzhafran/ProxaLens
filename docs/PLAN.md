# ProxyDebug — free Proxyman-style desktop app (Tauri + Rust + React)

## Context
You want a free, GUI alternative to Proxyman/Charles. `kostyay/httpmon` (MIT, Go, ~25k LOC incl. tests) is a terminal (Bubble Tea) MITM debugging proxy with most of the features needed. Decisions made: **rewrite the engine in Rust inside Tauri** (httpmon serves as the reference spec, not as code we ship), **macOS first**, **React + TypeScript** UI. Repo: `git@github.com:naufalzhafran/project-proxy.git`. Reference source: `git clone https://github.com/kostyay/httpmon.git` (read it, port behaviour and tests; keep MIT attribution if logic is copied).

## Status / handoff
Update this checklist as work lands so any agent or device can pick up where the last left off.
- [x] Assessment + plan (this file)
- [x] Phase 0: Scaffold. Tauri v2 + React-TS + Vite 8, Tailwind v4, zustand, TanStack Table/Virtual, vitest.
  - Deferred to Phase 2, where they're first used: shadcn/ui and Monaco.
- [x] Phase 1: Capture core + system proxy crash safety (layers 1–4).
  - Engine: `src-tauri/src/engine/{store,ca,proxy,sysproxy,guard}.rs`.
  - IPC: `commands.rs`, with flow batches every 100 ms over a Tauri `Channel`.
  - Basic UI: toolbar, virtualized flow list, detail panel, banners.
  - Watchdog: the app binary re-runs itself as `--guard <pid> <marker>`, so there's no separate sidecar to bundle.
  - Tests: 20 Rust unit, 6 Rust e2e (HTTP, HTTPS MITM, request body, truncation, upstream failure, stop), 5 vitest.
  - Not yet done:
    - DNS/connect/TLS timings: hudsucker doesn't expose them, so only TTFB and total time are recorded.
    - No e2e test for WebSocket capture.
    - The proxy listens on 127.0.0.1 only (LAN/device access is Phase 6).
    - Manual checks still pending on a real Mac (they change real settings, so not automated):
      - "Install certificate…" trusts the CA.
      - The system proxy toggle switches every service on and off.
      - `kill -9` of the running app restores the proxy through the watchdog.
      - A relaunch restores and shows the notice.
- [x] Phase 2: Core UI
  - Three-pane layout with resizable panes:
    - Sidebar: all traffic, pinned domains (stored in localStorage), and a domain → host tree. `baseDomain` in `src/lib/http.ts` is a heuristic, not the Public Suffix List.
    - Flow table: adds a Type column, tints failed and in-progress rows, and supports Home/End keys.
    - Inspector: request (Headers/Query/Cookies/Body/Raw) on top, response (Headers/Cookies/Body/Raw/Messages/Timing) below. The body viewer has JSON tree, text, hex and image preview modes.
  - Filter bar: a port of httpmon's quick and advanced syntax (`src/lib/filter.ts`: `s:`, `m:`, `ct:`, `re:`, `!` negation, plus long aliases), with method and status chips. Filtering runs client-side over `FlowMeta`, so there's no Rust `filter.rs` yet (an MCP server will need one in Phase 6).
  - Context menu: copy URL, cURL (port of `tui/curl.go`, with shell escaping), request or response body; pin a domain; show only a host. Copying uses `tauri-plugin-clipboard-manager`.
  - Onboarding wizard (certificate, then system proxy). It opens on first launch while the CA is untrusted, and again from the toolbar.
  - Toolbar "Proxy On/Off" switch. Shortcuts: ⌘F focuses the filter, ⌘K clears flows.
  - Tray (menu bar) icon (`src-tauri/src/tray.rs`): status line, system proxy toggle, show window, clear, quit. A `status-changed` event keeps the UI in sync.
  - Not adopted, by decision: shadcn/ui (plain Tailwind primitives in `components/ui.tsx` are enough) and Monaco for read-only bodies (too heavy; it moves to the Phase 5 script editor).
  - Not yet done:
    - The process column and "Apps" tree need process identification (Phase 6).
    - Compressed bodies show as binary until body decoding lands (Phase 3).
  - Tests: 28 vitest (the httpmon filter and cURL tests are ported). The UI was smoke-run in `tauri dev` with curl traffic; clicking through it and checking the tray menu still has to be done by hand.
- [x] Phase 3: Debug tools
  - Body decoding (`engine/decode.rs`): gzip/deflate (zlib or raw)/br/zstd and stacked encodings. Bodies are stored as received and decoded when `get_flow` asks. Truncated captures decode as a prefix, and output is capped at 50 MB. The inspector shows the encoding and the size on the wire.
  - HAR 1.2 export/import (`engine/har.rs`, port of `har/har.go`), with decoded bodies and base64 for binary.
    - A session (`.ppsession`) is a zstd-compressed HAR. Extra fields (state, error, client, truncation, WebSocket messages) go in a `_projectProxy` extension object.
    - Open replaces the list (only once the file has parsed); Import appends.
  - Repeat / Edit and Repeat / Compose (`engine/repeat.rs`): reqwest sends the request through our own proxy, trusting our CA, so it is captured; the new flow is found and selected.
  - Diff (`src/lib/diff.ts`): a Myers line diff of request and response (start line, sorted headers, pretty-printed JSON body), after `diff/diff.go`. Use "Mark for Comparison", then "Compare with #N".
  - Settings (`engine/settings.rs`, `settings.json`, applied live without a restart):
    - SSL Proxying include/exclude, with exclude winning. Hosts that aren't decrypted are tunnelled and listed as a `tunnel` CONNECT row.
    - Block list: 403 response, and CONNECT is refused; flow state `blocked`.
    - Allow list: only matching hosts are recorded or decrypted.
    - Host patterns are a port of `hostfilter` (`*.x.com` matches subdomains only).
  - Throttle (`engine/throttle.rs`, port of `throttle/`): presets for 3G/4G/Wi-Fi bandwidth plus latency, or custom values, optionally per host. Quick picker in the toolbar.
  - UI: a File menu (⌘O/⌘S), Compose (⌘N), Repeat (⌘R) and a Settings dialog (⌘,). File pickers come from `tauri-plugin-dialog`.
  - Tests: 42 Rust unit, 14 Rust e2e (decoding, block, allow, SSL tunnel, throttle, repeat), 37 vitest. The app was smoke-run with gzip and brotli traffic; clicking through the new dialogs still has to be done by hand.
- [ ] Phase 4: Rules & breakpoints
- [ ] Phase 5: Scripting
- [ ] Phase 6: Advanced

## Assessment of httpmon

**What it is:** `cmd/httpmon/main.go` wires together a set of `internal/` packages:
| Package | Role | Rust equivalent |
|---|---|---|
| `proxy/` (`proxy.go`, `interceptor.go`) | MITM via `go-mitmproxy`, addon hooks: Requestheaders → Request → Responseheaders → Response; decompress, run scripts, recompress, throttle | `hudsucker` (hyper + rustls + rcgen) `HttpHandler` |
| `store/` (`ringbuffer.go`, `types.go`) | In-memory ring buffer; `FlowMeta` (list row) + `FlowData` (headers/bodies), 5 MB body cap | `parking_lot::RwLock<VecDeque>` + `IndexMap` by id |
| `certutil/` | Generate CA (RSA, `mitmproxy-ca-cert.pem`), install with `security add-trusted-cert` | `rcgen` + `security` CLI via admin prompt |
| `browse/sysproxy_darwin.go` | Set/restore system proxy via `networksetup` for the active service | same commands from Rust |
| `scripting/` | goja JS: `onRequest/onResponse(ctx)`, `respondWith`, `breakpoint()`, `readFile`, YAML frontmatter + glob match | `rquickjs` (QuickJS) |
| `breakpoint/` | `Pause(hit) → Resume` blocking controller with subscribe channel | `tokio::sync::oneshot` per hit + Tauri event |
| `throttle/` | Bandwidth-limited reader + latency, presets 3g/4g/wifi | throttled `Stream` wrapper on body |
| `bodydecoder/` | gzip/br/zstd, protobuf & gRPC-Web → JSON with per-host .proto registries | `flate2`, `brotli`, `zstd`, `prost-reflect` + `protox` |
| `procinfo/` | Map client port → PID → process name (gopsutil) | `libproc` / `netstat2` crates |
| `filter/`, `hostfilter/` | Quick + advanced filters (`status:`, `method:`, regex), block/allow globs | straightforward port |
| `har/`, `diff/`, `tui/curl.go`, `compose`, `repeat` | HAR 1.2 export, diff, copy-as-cURL, compose/replay | Rust (HAR) + TS (cURL, diff UI) |
| `mcpserver/` | MCP tools for LLM agents | `rmcp` crate — later phase |

**Strengths to borrow:** clean separation of concerns, a well-defined script API, a solid test suite (e2e proxy tests, scripting tests) you can port as behavioural specs, and MIT licence (keep attribution if you copy logic/tests).
**Gaps vs Proxyman:** no GUI, no saved sessions, no GUI rule editors for Map Local / Map Remote / Rewrite (it uses scripts for everything), no WebSocket frame inspector, no iOS-device setup flow, no Windows support, HTTP/2 handled only by the library.

## Feasibility verdict
**Feasible, medium-to-high effort.** Every hard piece has a mature Rust crate (`hudsucker`, `rcgen`, `rustls`, `rquickjs`, `prost-reflect`), and Tauri v2 handles IPC, the tray, the updater and signing. Main risks:
1. **IPC throughput**: thousands of flows per second can flood the webview. Mitigate by batching list deltas (about every 100 ms) through a Tauri `Channel`, sending only `FlowMeta`, fetching bodies lazily, and virtualizing the table.
2. **CA trust + system proxy need admin rights**: run `security` / `networksetup` through `osascript ... with administrator privileges`. Restore the proxy on quit **and** after a crash (write a marker file and check it on startup).
3. **Pinned apps / HTTP/3**: these can't be intercepted, same as Proxyman. Add an SSL-bypass list and passthrough tunnelling.
4. **Distribution**: a "free" app still needs an Apple Developer ID ($99/yr) for notarization; otherwise users have to bypass Gatekeeper.
5. **Toolchain**: Rust isn't installed (`rustup` is needed). Go and Node 24 are already present.

Rough effort for one developer: MVP (Phases 0–3) in about 4–6 weeks, Proxyman-level parity (Phases 4–6) in about 3–4 months.

## Architecture
```
proxydebug/
  src-tauri/
    src/main.rs, lib.rs          # Tauri builder, state, commands, tray
    src/commands.rs              # #[tauri::command] API surface
    src/engine/
      mod.rs                     # Engine: start/stop, config, shared state (Arc)
      proxy.rs                   # hudsucker HttpHandler/WebSocketHandler
      store.rs                   # ring buffer + flow types (port of store/types.go)
      ca.rs                      # rcgen CA gen/load, trust check/install
      sysproxy.rs                # networksetup set/restore (+ crash recovery)
      rules.rs                   # Map Local, Map Remote, Rewrite, Block/Allow, SSL bypass
      script.rs                  # rquickjs engine (port of scripting/)
      breakpoint.rs              # pending hits, oneshot resume
      throttle.rs, decode.rs, proto.rs, procinfo.rs, har.rs, filter.rs
      session.rs                 # save/load .pdsession (SQLite via rusqlite or zstd JSON)
  src/                           # React + TS (Vite)
    api/ (typed invoke wrappers, flow channel subscription)
    state/ (zustand store: flows, selection, filters)
    components/ FlowTable (TanStack Table + Virtual), Sidebar (host/app tree),
                Inspector (Headers/Body/Raw/Preview/Timing tabs, Monaco read-only),
                Composer, DiffView, BreakpointModal, RulesEditor, ScriptsEditor, Settings
```
- **Data flow:** the hudsucker handler builds `FlowMeta` and pushes it to `store`, then to a batched `Channel<Vec<FlowDelta>>`, and the React store merges it in. The UI calls `get_flow(id)` for bodies (decoded server-side, with pretty JSON/proto optional).
- **Rule pipeline order** (mirrors httpmon's interceptor): host filter/SSL bypass → block → breakpoint(request) → Map Local / scripts `onRequest` → Map Remote → upstream → decode → Rewrite / scripts `onResponse` → breakpoint(response) → throttle → client.
- Rules are stored as JSON in `~/Library/Application Support/<app>/rules.json`. Scripts keep httpmon's file format (JS + YAML frontmatter) so they stay compatible.

## System proxy crash safety
httpmon (`internal/browse/sysproxy_darwin.go`) keeps the saved proxy settings only in memory and restores them through `defer` and signal handlers. A force-kill (`kill -9`), a hard crash or a power loss leaves the Mac pointing at a dead proxy. We use layers instead, so no single failure can strand the user:

1. **Save before changing.** Read the HTTP and HTTPS proxy settings (`networksetup -getwebproxy` / `-getsecurewebproxy`) for **every** network service, not just the active one (Wi-Fi can switch to Ethernet mid-session). Write them to `~/Library/Application Support/<app>/sysproxy.marker.json` together with our host:port, the app's PID and a timestamp. Write atomically (temp file, fsync, rename), and only then change the proxy.
2. **Restore on every exit the app can see:** Tauri `RunEvent::Exit` and `ExitRequested`, Quit from the window or tray, SIGTERM/SIGINT/SIGHUP (via `tokio::signal`), `std::panic::set_hook`, and sleep/power-off notifications. Delete the marker after a successful restore.
3. **Watchdog for hard crashes.** When the proxy is switched on, start a tiny sidecar, `proxydebug-guard --pid <app> --marker <path>` (about 100 lines of Rust). It waits on `kqueue` `EVFILT_PROC`/`NOTE_EXIT` for the app's PID, and when the app dies for any reason it restores the settings from the marker and exits.
4. **Check again on next launch.** If the marker exists and its PID is no longer running, restore. Only touch a service whose settings **still point to our host:port**, so manual changes made since are left alone. Then show a notice: "Restored your network proxy settings from a previous session."
5. **Optional auto-config mode (setting, off by default).** Use `networksetup -setautoproxyurl` with a PAC file served by the app: `PROXY 127.0.0.1:<port>; DIRECT`. If the app dies, most clients fall back to a direct connection by themselves. Trade-offs: some apps ignore PAC files, and traffic can silently bypass the proxy.

**Admin rights:** proxy changes may ask for an admin password, and a watchdog can't answer a prompt when nobody is at the Mac. MVP: run `networksetup` directly, and fall back to `osascript ... with administrator privileges` if it fails. Phase 6: install a privileged helper once through `SMAppService` (a launchd daemon). It handles set/restore without prompts and takes over the watchdog role.

**Rollout:** layers 1–4 in Phase 1; the privileged helper and auto-config mode in Phase 6.

## Implementation phases
**Phase 0: Scaffold (2–3 days)**
- Install `rustup`. Run `npm create tauri-app@latest` (React-TS, Tauri v2) and `git init`.
- Add crates: `hudsucker`, `tokio`, `rcgen`, `rustls`, `serde`, `parking_lot`, `uuid`, `thiserror`, `tracing`.
- Add JS dependencies: `@tanstack/react-table`, `@tanstack/react-virtual`, `zustand`, `@monaco-editor/react`, plus a UI kit (shadcn/ui + Tailwind).

**Phase 1: Capture core (1–2 weeks)**
- `ca.rs`: generate or load the CA, export `.pem`/`.cer`, check trust with `security verify-cert`, install through an admin prompt.
- `proxy.rs`: HTTP + CONNECT MITM on a configurable port (default 9090), HTTP/1.1 + HTTP/2 upstream, WebSocket passthrough with frame capture, 5 MB body cap.
- `store.rs`: port `FlowMeta`/`FlowData` and the ring buffer; add timing fields (DNS/connect/TLS/TTFB when available).
- `sysproxy.rs` + `proxydebug-guard` watchdog: toggle the system proxy safely, using the layered restore strategy described in **System proxy crash safety** below.
- Commands: `start_proxy`, `stop_proxy`, `get_status`, `list_flows`, `get_flow`, `clear`, `install_ca`, `set_system_proxy`.

**Phase 2: Core UI (1–2 weeks)**
- Three-pane layout: sidebar (Apps / Domains tree, pinned), virtualized flow table (method, status, host, path, process, size, duration, time, colour coding), and inspector (request/response Headers, Query, Cookies, Body with JSON tree / hex / image preview, Raw).
- Filter bar: port httpmon's `filter/quick.go` and `advanced.go` syntax, plus method/status chips.
- Context menu: copy URL, copy as cURL (port `tui/curl.go`), copy response, pin domain.
- Onboarding: certificate setup wizard and a "Proxy On/Off" toolbar toggle; tray icon.

**Phase 3: Debug tools (1–2 weeks)**
- Body decoding: gzip/deflate/br/zstd.
- HAR import/export (port `har/har.go`); save/open sessions.
- Repeat / Edit-and-Repeat / Compose (via `reqwest`, captured as new flows); diff two flows.
- Block/Allow lists and an SSL Proxying include/exclude list (Proxyman-style per-domain SSL enable).
- Network throttle presets (port `throttle/`).

**Phase 4: Rules & breakpoints (2–3 weeks)**
- GUI rule editors: Map Local (file or inline body), Map Remote (host/path rewrite), Rewrite (header/body/status regex), Block.
- Breakpoints: a pattern list plus a "Breakpoint this request" context action. The engine awaits a `oneshot`, the UI shows an editable modal (headers/body/status) with Execute / Abort / Timeout.

**Phase 5: Scripting (2 weeks)**
- `rquickjs` engine that reproduces httpmon's `ctx` API (`method/url/headers/body/blocked`, `respondWith`, `breakpoint()`, `readFile` sandboxed to the scripts dir, a timeout interrupt).
- Port the `scripting/*_test.go` cases as Rust tests.
- Monaco script editor with templates, enable toggles and an error console.

**Phase 6: Advanced (ongoing)**
- Protobuf / gRPC-Web decode (`prost-reflect` + `protox`, with per-host registries as in `bodydecoder/host_registry.go`).
- Process identification (`libproc`: port → PID → app name and icon).
- WebSocket message inspector.
- iOS device / simulator setup (serve the cert at `http://proxy.local/cert`, show the LAN IP and a QR code).
- MCP server (`rmcp`), mirroring `mcpserver/` tools.
- Privileged `SMAppService` helper for prompt-free proxy switching and watchdog duty; optional PAC auto-config mode.
- Later: Windows support (WinINet proxy, `certutil` trust store).
- Later: a signed, notarized DMG release built by GitHub Actions (`tauri-action`), plus the Tauri updater. This was taken out of Phase 3.

## Verification
- **Unit:** `cargo test` for store, filter, rules, decode, script (ported from httpmon's tests); `vitest` for UI state and cURL generation.
- **Integration:** a Rust test harness starts the engine on a random port with a local `hyper` test server and a test CA, then sends requests through `reqwest` with the proxy set. Assert flow capture, HTTPS MITM, Map Local, Rewrite, breakpoints resume/abort, and throttle timing (mirrors `internal/proxy/*_e2e_test.go`).
- **Manual:** `npm run tauri dev`, then:
  - Install the CA and enable the system proxy, and confirm `curl -x http://127.0.0.1:9090 https://httpbin.org/get` and Safari/Chrome traffic appear live.
  - Check that the system proxy is restored after Quit and after `kill -9`.
- **Crash-safety tests (Phase 1):**
  - Switch the proxy on, `kill -9` the app, and confirm the watchdog restored every service and deleted the marker.
  - Leave a stale marker behind (dead PID, settings still ours), launch the app, and confirm it restores and shows the notice.
  - Leave a stale marker but change a service's proxy by hand first, launch the app, and confirm that service is left alone.
  - Unit-test parsing of `networksetup -get*proxy` output and marker read/write, with `networksetup` mocked behind a trait.
  - Load-test with `hey`/`oha` (10k requests) and confirm the UI stays responsive.
