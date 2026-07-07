# Build brief — Composer Phase 3: persistence (save / reopen compositions)

Read `docs/COMPOSER-SPEC.md` FIRST — "Phase 3 (persistence)" + "Songsmith integration" +
requirement #4 under "Flexibility / import-export" are the authority. Build on the current tree
(latest commit 5a56850).

## Outcome
Compositions survive the app closing. In the Composer: **Save** (uses the existing name field),
a **library of saved compositions** (list → open, delete), and clean new-blank behavior. Both
blank sketches AND full-song imports (`?song=<id>`) are saveable; a composition remembers which
song it came from (nullable link).

## Data layer (Rust core — mirror the `progression` table/tools pattern exactly)
- `models.rs`: `CompositionRow { id, name, song_id: Option<String>, data: String /*Composition
  JSON blob*/, created_at, updated_at }` (ts-rs exported; follow existing derive/attr style).
- `db.rs`: `composition` table in `migrate` (idempotent, like the others; `song_id` nullable TEXT);
  CRUD: `list_compositions` (id, name, song_id, updated_at — newest first; a light listing is fine),
  `get_composition`, `save_composition(id: Option, name, song_id: Option, data) -> CompositionRow`
  (id None = insert with new id; Some = update in place, bump updated_at — the upsert semantics
  music-kb used), `delete_composition`. Validate `data` is parseable JSON before saving (reject
  garbage; the frontend sends the zod-validated blob).
- `tools.rs`: register `list_compositions` / `get_composition` / `save_composition` /
  `delete_composition` in the registry + dispatch (delete = destructive:true), so Claude can read/
  write sketches over MCP — same shape as the progression tools. The registry↔mock parity test will
  force mock entries; add them.
- `app/src-tauri/src/lib.rs`: the four Tauri commands, registered.

## Frontend (`components/compose/Composer.tsx` + `routes/ComposerRoute.tsx`)
- The composition already has `id`/`name`; wire:
  - **Save** button next to the name field: serialize the current comp (the reducer state) through
    `CompositionSchema` (zod — already exists in `schema.ts`) and `save_composition`. First save of
    an unsaved sketch inserts (keep the client id or adopt the row id — pick one and be consistent);
    subsequent saves update in place. Show saved/dirty state (track dirty via reducer edits since
    last save — simplest reliable approach is comparing a serialized snapshot ref).
  - **Open** — a compact library panel/dropdown (list from `list_compositions`: name, updated_at,
    song badge when song_id set) → open loads via `parseStoredComposition` + `reidentify` (BOTH
    already exist and are the designed load seam — use them), replacing the current comp (confirm
    if dirty). Delete (×) per row with confirm.
  - **New blank** keeps working (confirm if dirty).
  - Full-song import (`?song=`): saving stores `song_id`; reopening from the library restores the
    full-song timeline INCLUDING sections/lyrics (they're in the blob — verify round-trip).
- `ipc/api.ts` + `ipc/mockApi.ts` parity (mock keeps an in-memory list; seed one saved sketch).

## Tests + visuals
- core: save→get round-trip preserves the blob byte-for-byte; update-in-place bumps updated_at and
  doesn't duplicate; delete removes; invalid-JSON data rejected; list ordering. Keep ALL existing
  tests green (31 core + 2 app).
- Frontend behavior via the visual test: save a sketch, see it in the library panel, reopen it —
  screenshot the open-library panel (`composer-library.png`). Kill port 5173 first.
- Update `docs/COMPOSER-SPEC.md` (Phase 3 → ✅ DONE + built-block) and the README Composer bullet
  (drop "in-memory for now"; saving is real, export-back still upcoming).

## Definition of done
- `cargo build` + `cargo test` green (paste lines); `cd frontend && npm run build` succeeds;
  visual test green with the new screenshot. Do NOT commit — the parent verifies and commits.

## Rules
- Files are NOT the store here — libSQL is (per spec; unlike skills). Reuse
  `parseStoredComposition`/`reidentify`/`CompositionSchema` — do not write a second
  serializer/validator. Don't break blank-sketch mode, full-song import, the lyric sheet, freeze,
  or the lane interactions. Match existing style everywhere (registry pattern, CRUD idioms, dark
  neon aesthetic). Run `cargo test` before the frontend build so ts-rs emits `CompositionRow`.
