# Claudexor for Linux

A native egui client for the [Claudexor](https://github.com/razzant/claudexor)
daemon (protocol major 3, checked against claudexor 3.13.0). See `PLAN.md` for
scope and design.

## Build and run

```sh
cargo build --release
./target/release/claudexor-linux
./packaging/install.sh          # ~/.local/bin + app launcher entry + icon
```

Sign in from the app: Accounts (bottom-left) → **Log in** next to a harness whose engine
offers in-app login (Claude today): open the link, paste the code back.

The daemon must be running (`claudexor daemon start`); the app finds it through
`~/.claudexor/v3/daemon/control-api.json` (or `$CLAUDEXOR_CONFIG_DIR`) and
reconnects on its own.

## Tools

| Command / env | Purpose |
|---|---|
| `claudexor-linux --record DIR` | dump live read-only API responses + one run SSE log as fixtures |
| `claudexor-linux --send PROMPT` | headless write path: new Ask thread, one turn, follow its stream |
| `CXL_FRAMESTATS=1` | print GL renderer and GPU/CPU ms per frame (forces continuous repaint) |
| `CXL_NO_FROST=1`, `CXL_NO_REFRACT=1` | switch off the L1 / L2 glass layers |
| `CXL_REPAINT_DEBUG=1` | print why each frame was repainted (idle-cost debugging) |
| `CLAUDEXOR_REDUCE_TRANSPARENCY=1` | start with solid surfaces (also a toggle in the ⚙ menu) |

`cargo test` decodes the upstream wire fixtures and `tests/fixtures/live/`
(recorded from a real daemon), and covers the SSE parser, reconnect resume
over a real socket, the transcript reducer, secure token reads, glyph coverage
and WCAG contrast of the theme tokens.

## Hyprland glass

The window is transparent; for the desktop to blur through it, keep
`decoration:blur` enabled (default). Frosted popovers and refracting chrome
work inside the window on any compositor.

## Measured on this laptop (Intel HD 6000, Mesa 26.2, 1180×780)

| Budget (PLAN §7) | Target | Measured |
|---|---|---|
| idle CPU | 0% | 0.03% (1 tick / 30 s: SSE heartbeat) |
| cold start → window | < 300 ms | 220 ms |
| frame with glass (GPU) | < 4 ms | 1.6 ms avg, 1.9 ms max |
| release binary | < 15 MB | 10.6 MB |
| idle RSS | < 80 MB | 94 MB VmRSS — 65 MB of it is Mesa's shared `libLLVM`/`libgallium`; app-private 22 MB, PSS 36 MB |
