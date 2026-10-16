# Changelog

## 0.3.0 (2026-10-16)

A fresh, more colorful look.

- A violet accent replaces teal, with retuned light and dark palettes.
- Sidebar rows have colored icon tiles: apps show their initial, domains a globe, and each name keeps its own color.
- HTTP methods appear as tinted badges in the flow table and inspector.
- The selected flow is a solid accent row, and the selected sidebar row a solid pill.
- The flow table footer tallies the shown flows by status class (2xx, 3xx, 4xx, 5xx and errors).
- Tabs, body view switches, filters and the recording toggle are rounded pill controls, and toolbar icons are colored by tool.
- Menus, popovers and dialogs float on soft shadows.

## 0.2.3 (2026-10-16)

Hotfix for resizing panes and columns.

- Pane dividers are easier to grab: the hit area is about 10px wide while the drawn line stays 1px.
- Dragging a divider is smoother. It updates once per frame and no longer re-renders the whole window.
- During a drag the resize cursor stays put, and rows and buttons under the pointer no longer flicker on hover.
- Hovering a divider waits briefly before highlighting it, so moving the pointer across one doesn't flash.
- Column resize handles are centred on their line in the column gap.
- Double-clicking a pane divider resets it, as it already did for columns.
- Right-clicking a divider no longer starts a drag, and switching away from the window ends any drag in progress.

## 0.2.2 (2026-10-15)

- Host patterns allow `*` and `?` anywhere.
- Release notes are generated from the commits since the previous release.

## 0.2.1 (2026-10-15)

- The whole app bundle is ad-hoc signed, so macOS no longer reports it as damaged.
- Releases are published directly, so the updater sees them as the latest.

## 0.2.0 (2026-10-14)

- Auto-update through the Tauri updater and GitHub releases.

## 0.1.0 (2026-10-14)

First release: HTTP/HTTPS capture with a crash-safe system proxy, flow table and inspector, rules and breakpoints, scripting, protobuf and gRPC decoding, per-app attribution, a WebSocket inspector, device setup with PAC mode, and an MCP server.
