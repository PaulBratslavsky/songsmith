# Songsmith Studio

**Claude Code for music producers.** A desktop cockpit that co-writes a *song mock* with
Claude — structure, chords, and lyrics — fast, then hands it off to your pipeline. Claude
is the engine (your Claude Code subscription, via the `claude` CLI + an MCP server); the app
is the harness that wraps it with songwriting skills, a per-project style preset, a real
chord/theory toolkit, and a structured spark-to-mock workflow.

A producer sets a reusable **style preset** (genre, mood, influences, key/tempo feel, vocal
range, themes), starts a song, and walks a **song spec** — a stage checklist:

> **Concept → Structure → Chords → Lyrics → Generation Prompt**

At each stage Claude drafts the part; you **one-click run**, **chat to refine** (per-stage,
over MCP), or **hand-edit**. Every approved part carries forward, so the song stays coherent.
The output feeds a real pipeline:

> **Songsmith** (mock: structure + chords + lyrics + generation prompt) → **Ableton** (play
> the voicings) → **an AI music generator** (e.g. Suno, Udio — audio from lyrics + style) →
> **back into Ableton** (finish) → **🎧 drop the renders back in and ★ pick the keeper.**

> Forked from the YT Creator Studio harness and reshaped for songwriting. Planning docs in
> [`resources/`](resources/). License: **GPL-3.0**.

## Highlights

- **Claude is the single engine** — no local model. Stage runs and the Chat tab both drive
  `claude` headless (stream-json) with this app's MCP server attached, so Claude reads and
  edits song state through the same tool registry the UI uses.
- **Per-stage chat** — a collapsible "💬 Chat with Claude about this stage" on every stage;
  ask for a change and Claude edits that stage's artifact over MCP, and it reloads live.
- **Real chord engine** — the music-theory + voicing engine is ported from
  [music-kb](https://github.com/) (`frontend/src/lib/music`, GPL-compatible, `tonal`-backed):
  correct chord tones, **CAGED + open guitar voicings**, a voicing cycler, scales, inversions.
- **Chord Builder** (dedicated tab) — circle-of-fifths wheel, root/quality picker, a **full
  guitar fretboard** + piano voicing, **♪ playback** (Web Audio), save progressions to a
  reusable library, and **export an SVG chord chart** to play from. Import any saved
  progression straight into a song section.
- **Absolute-chord composer** — the Chords stage stores real chord names per section (never
  collapsed to scale degrees), editable by hand, by palette, by import, or by chat.
- **Composer** (dedicated tab) — a Hookpad-style **visual sketchpad**: drag chord blocks, melody,
  and bass notes onto a shared scale-degree grid and loop it through the per-voice synth; change the
  key and everything transposes. A blank sketch is 8 bars; from any song, **"Open in Composer"** loads
  the *whole song* onto one long, scrollable, multi-section timeline — sections laid end-to-end, the
  real chords on the chord lane with the lyric line under each, melody + bass editable across the song.
  Degree-based + key-relative (in-memory for now; save/export-back are upcoming phases).
  See `docs/COMPOSER-SPEC.md`.
- **Suno-accurate prompts** — the Generation Prompt stage follows current best practice: key +
  tempo in the **style** line, chords as `[Am]` tags **inline in the lyrics** (not sung).
- **Freeze sections** — 🔒 lock any section in Structure / Chords / Lyric Spec / Lyrics and
  regenerating that stage keeps it byte-for-byte; only unlocked sections are rewritten.
- **Paste finished lyrics** — 📋 drop completed lyrics into the Lyrics stage (or "New from lyrics"
  on the Library) and they're parsed into sections with the words kept **verbatim** — header split
  first, Claude only ever marks boundaries on unlabeled text (validated line-by-line against your
  paste) — then the Structure stage is back-filled to match.
- **Final renders** — reference your generated audio versions on disk (multiple takes), play
  them, reveal in Finder, ★ pick the winner. Audio stays in your DAW-friendly folder.
- **Extensible skills** — every stage's songwriting method is an editable skill; add custom
  ones (Nashville numbers, genre structures, lyric techniques, prompt templates).

## Architecture

```
songsmith-studio/
├── core/                 # Rust core — owns all state
│   └── src/
│       ├── models.rs     # StylePreset, Song, Stage, Artifact, Skill, Progression, Render
│       ├── db.rs         # libSQL schema, migrations, journaled artifacts, CRUD, skill seed
│       ├── tools.rs      # ToolSpec registry + dispatch — one registry, three callers
│       ├── agent.rs      # run_stage + call_claude (the claude CLI, stream-json)
│       └── skills/       # built-in songwriting skill markdown
├── app/src-tauri/        # Tauri v2 shell — IPC commands, chat_send, dialog/opener plugins
├── frontend/             # React + TanStack, single styles.css
│   └── src/
│       ├── routes/       # /, /presets, /song/$id, /builder, /composer, /skills, /chat, /settings
│       ├── components/   # StageChecklist, Composer, StageChat, ChordBuilder, GuitarView,
│       │                 #   FinalRenders, ChatPanel, compose/ (visual Composer), …
│       ├── lib/music/    # ported music-kb engine (theory, voicings, instruments, compose) + tonal
│       └── ipc/          # typed bindings + generated/ types (ts-rs) + mockApi.ts
└── mcp-shim/             # stdio MCP server over the shared DB (server name: "songsmith")
```

The **Rust core owns all state**. The frontend is a view layer; the Tauri shell and the MCP
shim are thin layers over `core`. The registry in `core/src/tools.rs` is the single contract
shared by the UI, the agent, and Claude over MCP.

## Prerequisites

- Rust 1.85+ · Node 20+ · cmake
- **[Claude Code](https://claude.com/claude-code)** installed and signed in — it's the engine.
  Run `claude` once to authenticate. **Settings → Claude engine** shows the detected status.

## Run

```bash
cd frontend && npm install && cd ..           # install JS deps (once)
make dev                                        # tauri dev — boots Songsmith
# or build + install into /Applications and launch:
make install
```

The frontend also runs standalone in a browser against an in-memory mock (`npm run dev` in
`frontend/`) for UI work without the backend.

## Connecting your Claude account (in-app)

Songsmith drives Claude through your **claude.ai subscription** — never an API key. Everything
happens in **Settings → Connect Claude account**; no terminal required:

- **Status** — a badge shows `● connected as <account> (<subscription>)`, `⚠ not signed in`, or
  `⚠ API key set — connectors off`, plus a one-line hint about your connectors.
- **Log in** — opens the claude.ai OAuth page in your browser and streams the CLI's progress into
  a small log box. Finish signing in, copy the **authentication code** claude.ai shows, paste it
  into the box, and click **Submit** — status re-checks automatically. (**Cancel** abandons a
  stuck attempt.)
- **Log out** — signs out of your claude.ai account (`claude auth logout`), then re-checks status.
- **Re-check** re-reads the auth state; **Test** asks Claude to reply `READY` to confirm the live
  path works end-to-end; **Connect tools ↗** opens the claude.ai connectors page.

> Keep `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` **unset**. The app strips them from every
> `claude` process it spawns — that strip is exactly what keeps your subscription login and
> connectors active. Setting a key forces API billing and turns connectors off.

## Let Claude drive it (MCP)

The **Chat** tab and the per-stage chats wire this automatically. To drive the harness from
your own terminal:

```bash
cargo build -p mcp-shim --release
SONGSMITH_DB="$HOME/Library/Application Support/com.songsmithstudio.desktop/songsmith-studio.db" \
  claude mcp add songsmith --scope user -- /path/to/target/release/mcp-shim
```

Claude can then `list_songs`, `run_stage`, `save_artifact`, `save_progression`, `add_render`,
etc. — the same tools the UI uses. **Settings → MCP** shows the ready-to-run command.

## Make targets

```bash
make dev      # tauri dev (hot reload)
make install  # build .app → /Applications → launch
make build    # .app only
make dmg      # distributable .dmg
make test     # cargo test -p song_core
make types    # regenerate the TS types from Rust
```

TypeScript types are generated from the Rust models (ts-rs) — never hand-edit
`frontend/src/ipc/generated/`. A test enforces **mock-parity**: every registry tool exists in
`mockApi.ts`.

## Status

The core product is complete and installable: the full spark-to-mock loop runs end to end with
Claude, the chord engine + builder are wired in, prompts follow Suno best practice, and final
renders close the pipeline. Remaining polish: an inversion control in the Builder, deleting the
superseded interim chord helpers, a per-section lyrics editor, and Apple code-signing for the
`.dmg`.

## License & attribution

Forked from the **YT Creator Studio** harness and reshaped for songwriting. Planning docs live
in [`resources/`](resources/).

Licensed under **GPL-3.0** — see [`LICENSE`](LICENSE).
