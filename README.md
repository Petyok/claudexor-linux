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

The app finds the daemon through `~/.claudexor/v3/daemon/control-api.json` (or
`$CLAUDEXOR_CONFIG_DIR`), starts it once with `claudexor daemon start` if it is not
running (`CXL_NO_AUTOSTART=1` to opt out), and reconnects on its own.

## What it does

- **Threads**: create, search (Ctrl+K), rename / archive / trash / restore / delete
  forever (right-click a thread), Alt+↑/↓ to move between them.
- **Conversation**: answers as markdown, a receipt per turn (status, harness, the
  model that actually answered, time, cost, tools), live activity (thinking, tool
  rows) streamed over SSE and resumed without duplicates after an engine restart,
  refused / failed cards with the engine's own message, interactive questions.
  Each turn's outcome banner opens run details: phases, access, auth route, web
  evidence, context continuity, budget, plan checklist, Best-of candidates,
  review findings, sub-runs and warnings.
- **Workspace** (per thread): Changes, Outputs and Evidence of every run in one
  place, filterable to one turn.
- **Changes**: per Agent turn, the patch as a per-file diff with line numbers, **Apply patch** /
  **Apply as branch**, **Revert** to the pre-turn state, **Apply thread** for
  isolated threads, and a decision bar for blocked runs (accept risk, rerun with
  feedback, override needs-human with a confirm). Produced files show PNG/JPEG
  thumbnails and open in an in-app viewer (images, text, rendered markdown) or
  the desktop's default app.
- **Plan**: readiness chip, answer the plan's open questions (sent as a follow-up
  plan turn), **Implement plan** / **Implement anyway** (recorded override).
- **Composer**: Ask / Plan / Agent, project, harness, model, effort, account pin,
  attachments (file chooser via zenity/kdialog, screen region via grim + slurp),
  Send / Stop / Retry. **Options**: access (a full-access grant takes an explicit
  confirmation), Single / Best-of (explicit pool, per-harness models) / Until
  clean / Create (with a typed test command), max attempts, Delegate, Browser,
  review and reviewer panel, protected-path approvals,
  Plan Council, Ask deep scan, web mode, auth route, budget cap, isolated
  workspace.
- **Accounts**: per-account readiness and quota windows, Enabled toggle, in-app
  **Log in** (open the link, paste the code back), Add account, Remove.
- **Settings** (gear in the top-right toolbar): routing goal, paid fallback, primary harness, auth
  route, environment; per-run budget cap and question timeout; Harness Doctor
  with readiness checks, Recheck and per-harness default model / effort; API
  keys (write-only) and full-access grants with Revoke; engine concurrency.
- **Guard rails**: Send is blocked with the engine's own reason when the repo's
  git setup can't take the turn; a turn refused for full access offers "Allow
  full access & Retry" (with a confirm); harness, account and access picks are
  saved on the thread; a banner offers Relink when a project folder went
  missing; nested project roots are disclosed.
- **Onboarding and offline**: a "Set up a harness" prompt while none is ready,
  a note on the turn when the engine switches accounts at a quota limit, and the
  last thread list kept (owner-only, in `~/.cache/claudexor-linux`) and shown
  read-only while the engine is offline.
- **Desktop notifications** (notify-send) when a turn finishes or needs you and the
  window is not focused.
- **Glass**: compositor blur behind the window, frosted popovers, a refraction shader
  on the chrome; light/dark/system themes (System follows the desktop's
  colour-scheme setting) and Reduce transparency (Settings → Appearance).
- **Keyboard**: Enter sends, Shift+Enter is a new line, Ctrl+Enter sends or
  stops a running turn, Ctrl+N new thread (keeps the draft), Ctrl+K search,
  Alt+↑/↓ threads, Ctrl+. workspace panel.

## Tools

| Command / env | Purpose |
|---|---|
| `claudexor-linux --record DIR` | dump live read-only API responses + one run SSE log as fixtures |
| `claudexor-linux --send PROMPT` | headless write path: new Ask thread, one turn, follow its stream |
| `CXL_FRAMESTATS=1` | print GL renderer and GPU/CPU ms per frame (forces continuous repaint) |
| `CXL_NO_FROST=1`, `CXL_NO_REFRACT=1` | switch off the L1 / L2 glass layers |
| `CXL_REPAINT_DEBUG=1` | print why each frame was repainted (idle-cost debugging) |
| `CXL_NO_AUTOSTART=1` | never start the daemon from the app |
| `CLAUDEXOR_REDUCE_TRANSPARENCY=1` | start with solid surfaces (also a toggle in Settings → Appearance) |

`cargo test` decodes the upstream wire fixtures and `tests/fixtures/live/`
(recorded from a real daemon), and covers the SSE parser, reconnect resume
over a real socket, the transcript reducer, secure token reads, glyph coverage
and WCAG contrast of the theme tokens.

## Desktop blur

The window is transparent, so the compositor decides what shows through it.
On Hyprland keep `decoration:blur` enabled (the default). KDE blurs only windows that
request it, which this app does not do yet, and GNOME has no blur; there the
app draws its own gradient.
Frosted popovers and refracting chrome work inside the window on any
compositor.

## Performance

Reference measurements on a low-end integrated GPU (Intel HD 6000, Mesa 26.2),
1180×780 window:

| Budget (PLAN §7) | Target | Measured |
|---|---|---|
| idle CPU | 0% | 0.03% (1 tick / 30 s: SSE heartbeat) |
| cold start → window | < 300 ms | 220 ms |
| frame with glass (GPU) | < 4 ms | 1.6 ms avg, 1.9 ms max |
| release binary | < 15 MB | 14.6 MB (with AccessKit, PNG/JPEG previews and icons) |
| idle RSS | < 80 MB | 94 MB VmRSS, of which 65 MB is Mesa's shared `libLLVM`/`libgallium`; app-private 22 MB, PSS 36 MB |

## Credits

Fonts are bundled: Inter and JetBrains Mono (SIL OFL), and a subset of the
Lucide icons (ISC, `assets/fonts/LICENSE-Lucide.txt`).
