# Claudexor for Linux: egui MVP plan

A native Linux client for the Claudexor daemon, written in Rust with egui. It
should look good, use very little CPU and memory, and have a Liquid Glass-style
look.

## 0. Facts this plan rests on (checked against the claudexor repo, v3.13.0)

- The macOS app is only a client. It reads the discovery file
  `~/.claudexor/v3/daemon/control-api.json` (`{host, port, tokenPath}`; honours
  `$CLAUDEXOR_CONFIG_DIR`), then calls `http://host:port/v2/...` with
  `Authorization: Bearer <token>`. The daemon and CLI already run on Linux.
- There are 84 endpoints, listed in `docs/reference/endpoints.json`, plus 165
  JSON Schemas in `packages/schema/generated/`.
- Compatibility is checked with `POST /v2/handshake` `{protocolMajor, client}` →
  `{compatible, protocolMajor, engine, operationsPath, servingMode}`.
  The Swift code states that `protocolMajor` is the ONLY compatibility signal.
- Live updates come over SSE: `GET /v2/runs/:id/events` (replay + tail, resumable
  with `Last-Event-ID`) and `GET /v2/global/events`. Each frame is
  `BusEnvelope {seq, kind, event}`. Streams end with a terminal `end` event.
- The Mac app is about 43k lines of source (9.4k in `ClaudexorKit`). The MVP
  copies its behaviour, not its size.
- Its design system (`docs/DESIGN_SYSTEM.md`) is chat-first: thread list, then
  conversation, then a floating composer. Liquid Glass goes on the chrome only,
  frosted glass on cards, and code and dense text always sit on solid
  surfaces. Nothing animates when the app is idle. We follow the same rules.
- License: MIT.
- **Claudexor is not installed on this laptop yet** (`~/.claudexor` does not
  exist). Phase 0 installs it.

## 1. Scope

**In the MVP**
1. Connect to the daemon: discovery file, token, handshake, and clear offline
   and incompatible states.
2. Thread list: create a thread, pick a thread.
3. Conversation: past turns plus the live run timeline (markdown, tool calls,
   status).
4. Composer: prompt, harness and model picker, send, cancel, retry.
5. Accounts and quota popover (read-only, with a refresh button).
6. The glass look (§5).

**Not in the MVP, use the CLI for these:** sign-in and setup, SSH remote
threads, installing or updating the runtime, apply/delivery, artifacts and
gallery, Best-of and Council configuration, settings, trash.
Add them later in the order people actually ask for them.

## 2. Endpoints (15)

| Purpose | Call |
|---|---|
| handshake | `POST /v2/handshake` |
| thread list | `GET /v2/threads` |
| new thread | `POST /v2/threads` (`ControlThreadCreateRequest`: `workspace`, `primaryHarness`, `title`, `access`, …) |
| thread detail | `GET /v2/threads/:id` (turns + head) |
| send turn | `POST /v2/threads/:id/turns` (`ControlThreadTurnRequest`: MVP sends `prompt`, `primaryHarness`, `model`, `effort` only) |
| retry turn | `POST /v2/threads/:id/turns/:id/retry` |
| live timeline | `GET /v2/runs/:id/events` (SSE) |
| cancel | `POST /v2/runs/:id/control` |
| refresh list | `GET /v2/global/events` (SSE) |
| harnesses | `GET /v2/harnesses` |
| models | `GET /v2/harnesses/:id/models` |
| quota | `GET /v2/quota`, `POST /v2/quota` (refresh) |
| routing | `GET /v2/account-pools` |
| workspace picker | `GET /v2/projects` |
| optional: pending question | `POST /v2/runs/:id/interactions/:id/answer` |

## 3. Architecture

A single crate. No tokio, no generated types for all 165 schemas.

```
claudexor-linux/
  Cargo.toml
  src/
    main.rs        eframe setup, window, fonts
    api.rs         discovery, token, handshake, typed calls (ureq)
    sse.rs         SSE line parser + reconnect with Last-Event-ID
    model.rs       serde structs for the ~12 types used (tolerant decode)
    state.rs       app state + message reducer
    ui/
      theme.rs     tokens: colors, radii, spacing, fonts (light + dark)
      sidebar.rs   thread list
      thread.rs    conversation + timeline
      composer.rs  floating composer
      accounts.rs  quota popover
      glass.rs     glass surfaces (§5)
  tests/fixtures/  JSON + recorded SSE from a real daemon
```

- **Networking:** `ureq` on plain std threads. Each open stream gets one thread
  and sends messages to the UI through `std::sync::mpsc`, then calls
  `ctx.request_repaint()`. The UI never blocks.
  `ponytail:` one thread per stream. Switch to tokio only if we need more than
  about 20 streams at once, which we won't.
- **Types:** hand-written `serde` structs covering only the fields the UI
  reads. No `deny_unknown_fields`, and optional fields use `#[serde(default)]`.
  The daemon ships several releases a week, and a missing field has to degrade
  the display, not break it. Anything we don't model stays as
  `serde_json::Value`.
- **Crates:** `eframe` (glow backend, see §5), `egui_commonmark` (markdown),
  `ureq`, `serde`/`serde_json`, `dirs`. Fonts bundled in the binary: Inter for
  UI, JetBrains Mono for code.
- **Idle cost:** egui only repaints on input or when asked. Only SSE messages
  and animations request repaints. The target is 0% CPU when idle.

## 4. Visual design (the "nice looking" part)

- **Layout:** sidebar (thread list, 260 px), conversation (max width 760 px,
  centred), floating composer pinned to the bottom, and a status pill at the
  top right (daemon state + active account).
- **Tokens in `theme.rs`, the only place colours live:**
  - background: a deep neutral with a slow radial glow (dark), or warm off-white (light)
  - surfaces: `glass/chrome` (Liquid, §5), `glass/card` (frosted), `solid/code` (opaque, high contrast)
  - one accent colour, plus status colours for running, ok, failed, needs-you
  - radii 14/20/28 px, a 4 px spacing scale, 13/15/20 px type sizes
- **Timeline:** each turn is a card. The run status shows as a small pill with a
  pulse only while running, static otherwise. Tool calls are collapsed rows
  that expand into solid code blocks. The last turn stays visible without
  scrolling.
- **Motion:** 150–200 ms ease-out on hover, expand and open. Nothing moves when
  the app is idle.
- **Themes:** follow the system light/dark setting, and add a "Reduce
  transparency" toggle that turns every glass surface solid.

## 5. Liquid Glass: three layers, each with a fallback

References: [charlesgrassi.dev: Recreating Apple's Liquid Glass](https://charlesgrassi.dev/blog/apple-liquid-glass),
[kube.io: refraction with CSS/SVG](https://kube.io/blog/liquid-glass-css-svg),
[sorrell.info: lens effect](https://sorrell.info/blog/liquid-glass-lens-effect),
[dev.to: AGSL refraction shader](https://dev.to/wrack/liquid-glass-on-android-in-react-native-i-refracted-the-backdrop-with-a-real-time-agsl-shader-1dp4),
[backdrop-blur-egui](https://docs.rs/backdrop-blur-egui).

The key idea from all of these: glassmorphism blurs what's behind it, while
Liquid Glass *refracts* it. The edge bends the background like a lens, the
centre stays clear, and there's a bright specular rim.

**L0: desktop shows through the window (nearly free).**
The window is created transparent (`ViewportBuilder::with_transparent(true)`
plus a transparent clear colour), and the compositor blurs whatever is behind
it. Hyprland does this out of the box through `decoration:blur`, optionally
tuned with a `windowrule` for the app class. KDE needs the blur-behind
protocol. GNOME has no blur, so the app falls back to its own gradient
background.

**L1: frosted panels inside the window (a library).**
`backdrop-blur-egui` with the `grab-pass` feature (eframe on glow): call
`renderer.frost(ui, Surface{rect, blur_radius, tint, corner_radius, presence})`
*before* drawing a panel's content. It's used for cards and popovers. The rules
it imposes: frost before the foreground, fade with `Presence` (not
`multiply_opacity`), and a panel whose size changes uses last frame's rect.
The crate is pre-release (0.1/0.2). If it doesn't work with our eframe
version, fall back to L2's shader with refraction switched off.

**L2: refraction on the chrome (our own shader, about 300 lines of GLSL + Rust).**
A glow `PaintCallback` over the grabbed framebuffer. It's used only on the
composer, the status pill and the sidebar header:
1. A squircle SDF for the shape. Its gradient gives the edge normal.
2. Bevel refraction: push the sample UV inward along the normal, scaled by a
   curved profile that is strong at the rim and zero in the centre.
3. Chromatic dispersion: separate R/G/B offsets, very small (about 0.02×) so it
   reads as glass rather than a rainbow artefact.
4. A light blur (Kawase or 8-tap ring), a cool tint, a Fresnel rim and a thin
   specular line along the top edge.
5. Fade the refraction out over the outermost ~10 px, to avoid the "cracked
   glass" hairlines the dev.to article describes.
6. Repaint only when the content behind changes. No constant animation, per
   the design rules.

**Hard rule:** no glass behind code, diffs or dense text. Those always use
`solid/code`.

## 6. Phases and exit criteria

| # | Phase | Done when |
|---|---|---|
| 0 | **Spike** (1 day): install claudexor, start the daemon, curl handshake, record fixtures for every endpoint in §2 plus one full SSE run. Separately, an eframe-glow transparent window on Hyprland with one `frost()` panel. | fixtures in `tests/fixtures/`; screenshot of a blurred panel over the desktop; `protocolMajor` noted |
| 1 | **Client core:** `api.rs`, `sse.rs`, `model.rs` | `cargo test` decodes every fixture; the SSE parser handles CRLF, multi-line `data`, comments/heartbeats and resume |
| 2 | **Layout + theme:** sidebar, static thread view, tokens, fonts | real threads render from the daemon; light and dark both look right |
| 3 | **Live runs:** composer → turn, streaming timeline, cancel, retry, reconnect after a daemon restart | a real turn streams to the end; killing the daemon mid-run shows offline and resumes without duplicate events |
| 4 | **Accounts:** harness/model picker, quota popover | quota matches `claudexor quota` |
| 5 | **Glass:** L0 → L1 → L2, plus the Reduce transparency toggle | looks good over a busy wallpaper; text contrast ≥ 4.5:1 on glass |
| 6 | **Packaging:** `cargo build --release`, `.desktop` file, icon | starts from the app launcher |

Rough effort: phases 0–4 take about a week, and glass (phase 5) takes 2–4
days, mostly spent tuning the shader.

## 7. Budgets (measured, not guessed)

- idle CPU: 0% (`pidstat -p <pid> 1 30`)
- idle RSS: under 80 MB with glass on (`/proc/<pid>/status` VmRSS)
- release binary: under 15 MB
- cold start to first frame: under 300 ms
- one frame with glass: under 4 ms on the iGPU

## 8. Risks

| Risk | Mitigation |
|---|---|
| The daemon API changes (several releases a week) | Check `protocolMajor` at connect, decode leniently, re-record fixtures on each upgrade |
| `backdrop-blur-egui` is pre-release or incompatible with our eframe | Own glow shader (L2 without refraction), already budgeted |
| Compositor blur differs by desktop (GNOME has none) | L0 is optional; L1/L2 work inside the window on any desktop |
| Glass hurts readability | Solid surfaces behind code and dense text, a contrast check, a Reduce transparency toggle |
| egui text selection and rich text are weaker than native | Markdown via `egui_commonmark`; code blocks use a selectable `TextEdit` in read-only mode |
| Event `kind` names are undocumented | Take them from the recorded SSE fixture in phase 0; show unknown kinds as a generic collapsed row |
