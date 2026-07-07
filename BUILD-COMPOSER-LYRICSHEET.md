# Build brief — Composer: ChordPro lyric sheet + highlight sync + grid performance

Read `docs/COMPOSER-SPEC.md` FIRST — specifically the "Lyric display refinement (user, 2026-06-29)"
block (the authority for this build) — and `docs/AUDIT-2026-07-06.md` Tier-2 item 12 (the perf
finding this build also fixes). Build on the current tree (latest commit 4446632).

## Part A — Replace the lyric cascade with a ChordPro sheet below the timeline
Today `components/compose/LyricRow.tsx` pins each lyric line at its chord's tick INSIDE the lane
stack — lines cascade rightward, unreadable, and add one ~18px grid row per lyric line. Replace it:
- **Remove LyricRow from the lane stack.** The timeline keeps: SectionBand, melody NoteLane,
  ChordLane, bass NoteLane.
- **Add a lyric SHEET rendered BELOW the timeline** (normal vertical flow, not horizontally
  scrolling with the grid): grouped by section (section label header), each lyric line shown
  ChordPro-style — **chord name in accent color directly above the word it lands on** — left-aligned,
  wrapping naturally. REUSE the existing ChordPro rendering approach from `components/SongSheet.tsx`
  (and the `.cp-*` styles in styles.css) rather than re-implementing; extract a small shared
  presentational piece if that's cleanest. Only show the sheet in full-song mode (when the
  composition has sections/lyrics); blank sketches show no sheet.
- Data: the composition already carries `sections[]` and `lyrics[]` (tick-anchored lines) and chord
  spans with `name`. Map chord spans → the section + lyric line they fall in. If the lyric data on
  the Composition is insufficient for word-level chord anchoring, it's acceptable to anchor at
  line level (chord above the line's first word) — but check `compositionFromSong.ts` first: it
  builds lyrics from the Lyrics stage's ChordPro (which HAS word-level anchors via [chord] tags) —
  thread the word-level info through if reasonably cheap (extend the Composition lyric type).

## Part B — Two-way highlight sync (per the spec decision: section tint + exact chord)
- **Timeline → sheet:** selecting a chord block highlights its section (tint) in the sheet AND the
  exact chord/word (brighter mark). During playback, the highlight follows the playhead — the
  active chord's mark moves through the sheet; auto-scroll the sheet to keep the active section
  visible (gentle, only when it leaves view).
- **Sheet → timeline:** clicking a chord in the sheet selects the matching chord span in the
  timeline (same selection state the ChordLane uses).
- **Perf constraint (critical):** the playhead is a single CSS-calc line precisely so lanes don't
  re-render per tick — PRESERVE that. The sheet's active-chord highlight must re-render only when
  the ACTIVE CHORD changes (derive "current chord id" from the tick in the playback hook and only
  set state on change), never per tick. Keep the sheet memo'd by section.

## Part C — Grid performance (audit Tier-2 #12)
`NoteLane.tsx` renders 7 × totalTicks `<button>`s and `ChordLane.tsx` another totalTicks — a 48-bar
import ≈ 11,500 DOM nodes with fresh inline-style objects and per-cell hover handlers.
- Replace the per-tick background cells in BOTH lanes with: CSS `repeating-linear-gradient`
  gridlines (beat/bar emphasis like today) + **one pointer hit surface per lane** that computes
  `(tick, degree)` from `offsetX/offsetY` for click-to-add and hover-audition. `useSpanDrag.ts`
  already derives ticks from pointer geometry — reuse that math (extract a shared helper if useful).
- Keep the SPAN blocks (notes/chords) as real elements — drag/resize/select behavior must be
  IDENTICAL to today (pointer capture, clamps, audition on vertical drag).
- Scope selection re-renders per lane: pass each lane only its own selection
  (`selected?.kind === lane ? selected.id : null`) instead of the shared object.
- Also: extract the copy-pasted 10-line label-gutter style block (ChordLane/NoteLane/SectionBand —
  LyricRow dies in Part A) into a `.cmp-lane-label` class or a `<LaneLabel>` component.
- Delete the now-dead back-compat aliases `BARS`/`TOTAL_TICKS` in `lib/music/compose/types.ts` and
  the re-export in `playback.ts` (audit verified zero real importers), and fix the stale "128-tick"
  comments (ChordLane.tsx:1, NoteLane.tsx:3, styles.css:261).

## Definition of done
- `cargo build` + `cargo test` green (should be untouched — frontend-only build; paste the lines).
- `cd frontend && npm run build` succeeds.
- `python3 scripts/visual_test.py` updated + green: the full-song screenshot now shows the lyric
  SHEET below the timeline (chords above words, section headers); add a screenshot with a chord
  selected showing the sheet highlight; the blank-sketch screenshot still renders (no sheet, no
  regression). List the files. NOTE: kill any running dev app / free port 5173 first if needed.
- Interaction sanity: click-to-add notes/chords, drag-move, drag-resize, vertical-drag repitch,
  hover audition, Delete/Escape all still work (exercise what you can via Playwright; state what
  was verified manually vs by build only).
- Update `docs/COMPOSER-SPEC.md` (lyric refinement → DONE) and the audit doc (Tier-2 #12 → ✅ FIXED).
- Do NOT commit — the parent verifies and commits.

## Rules
- Reuse SongSheet's ChordPro rendering + existing `.cp-*` styles; don't re-implement ChordPro.
- Don't break blank-sketch mode, the freeze feature, or the memo'd-lane architecture. Playhead
  stays CSS-calc. Match the dark neon-green aesthetic. Off-limits: core/src/db.rs (being edited in
  parallel — skill precedence fix). Everything else in compose/ + styles + visual test is yours.
