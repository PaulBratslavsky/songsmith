// The ChordPro lyric SHEET below the Composer timeline (full-song mode
// only) — the readable replacement for the old tick-pinned LyricRow
// (COMPOSER-SPEC "Lyric display refinement"). Grouped by section, each
// lyric line rendered ChordPro-style: chord name in accent directly above
// the word it lands on, left-aligned, wrapping naturally. Reuses the
// existing `.cp-*` ChordPro styles (LyricsEditor / Sheet) — no ChordPro
// re-implementation, just the read-only view.
//
// Two-way sync:
//   timeline → sheet: the highlighted chord (selected block, or the chord
//     under the playhead) tints its section and marks the exact chord/word;
//     during playback the sheet gently scrolls to keep it in view.
//   sheet → timeline: clicking a chord selects the matching chord span
//     (same selection state the ChordLane uses).
//
// Perf: the sheet receives `activeChordId` (which the playback hook only
// sets when the chord CHANGES — never per tick), the top level is memo'd,
// and each section is memo'd with a section-scoped highlight prop, so a
// chord change re-renders at most two sections and a tick change renders
// nothing here at all.

import { memo, useEffect, useRef } from 'react';
import type {
  ChordSpan,
  LyricLine,
  LyricWord,
  Section,
} from '../../lib/music/compose/types';
import { spanAt } from '../../lib/music/compose/spans';

export type SheetWord = {
  text: string;
  /** Chord name shown above the word (accent). */
  chord?: string;
  /** Matching timeline chord-span id (click-to-select + highlight). */
  chordId?: string;
};
export type SheetLine = { words: SheetWord[] };
export type SheetSection = {
  id: string;
  name: string;
  lines: SheetLine[];
  /** Every chord-span id inside the section (drives the section tint). */
  chordIds: string[];
};

/**
 * Pure: fold the composition's sections + lyric lines + chord spans into
 * the sheet model. Word-level chord tags (LyricLine.words, threaded from
 * the Lyrics stage's ChordPro) are matched to this section's chord spans
 * by name IN ORDER, so repeated progressions resolve to successive spans;
 * a line with no word tags falls back to line-level anchoring (the chord
 * at the line's anchor tick, above its first word). Wordless sections
 * (instrumentals) render as a chord-only line.
 */
export function buildSheetModel(
  sections: Section[],
  lyrics: LyricLine[],
  chords: ChordSpan[],
): SheetSection[] {
  if (!sections.length) return [];
  const sorted = [...chords].sort((a, b) => a.start - b.start);
  return sections.map((sec) => {
    const secEnd = sec.startTick + sec.lengthTicks;
    const spans = sorted.filter((s) => s.start >= sec.startTick && s.start < secEnd);
    const srcLines = lyrics.filter((l) => l.tick >= sec.startTick && l.tick < secEnd);

    // Greedy in-order matching of chord tags → chord spans (by name).
    let next = 0;
    const claim = (name?: string): ChordSpan | undefined => {
      if (!name) return undefined;
      const wanted = name.trim();
      for (let j = next; j < spans.length; j += 1) {
        if (spans[j].name === wanted) {
          next = j + 1;
          return spans[j];
        }
      }
      // Tag repeats after the section's spans ran out — reuse the first
      // span with that name so highlight/click still land somewhere sane.
      return spans.find((s) => s.name === wanted);
    };

    const lines: SheetLine[] = srcLines.map((ln) => {
      const source: LyricWord[] = ln.words?.length
        ? ln.words
        : ln.text
          ? ln.text.split(/\s+/).map((text) => ({ text }))
          : [];
      const words: SheetWord[] = source.map((w) => {
        const span = claim(w.chord);
        return { text: w.text, chord: w.chord, chordId: span?.id };
      });
      // Line-level fallback: no word carried a tag → show the chord at
      // the line's anchor tick above the first word.
      if (!words.some((w) => w.chord) && words.length) {
        const at = spanAt(spans, ln.tick);
        if (at?.name) {
          words[0] = { ...words[0], chord: at.name, chordId: at.id };
        }
      }
      return { words };
    });

    // Instrumental (no lyric lines): one chord-only line, still clickable.
    if (!lines.length && spans.some((s) => s.name)) {
      lines.push({
        words: spans
          .filter((s) => s.name)
          .map((s) => ({ text: '', chord: s.name, chordId: s.id })),
      });
    }

    return { id: sec.id, name: sec.name, lines, chordIds: spans.map((s) => s.id) };
  });
}

function SheetSectionImpl({
  section,
  highlightId,
  tinted,
  onSelectChord,
}: {
  section: SheetSection;
  /** The highlighted chord id IF it lives in this section, else null. */
  highlightId: string | null;
  tinted: boolean;
  onSelectChord: (id: string | null) => void;
}) {
  return (
    <div className={'cmp-sheet-section' + (tinted ? ' active' : '')}>
      <div className="cmp-sheet-head">{section.name}</div>
      <div className="cp-lyrics">
        {section.lines.map((line, li) => (
          <div key={li} className="cp-line">
            {line.words.map((w, wi) => (
              <span key={wi} className="cp-word">
                {w.chord ? (
                  <button
                    type="button"
                    className={
                      'cp-chord set' +
                      (w.chordId && w.chordId === highlightId ? ' current' : '')
                    }
                    data-cid={w.chordId}
                    disabled={!w.chordId}
                    title={w.chordId ? 'Select this chord in the timeline' : undefined}
                    onClick={() => w.chordId && onSelectChord(w.chordId)}
                  >
                    {w.chord}
                  </button>
                ) : (
                  <span className="cp-chord" aria-hidden="true" />
                )}
                {w.text ? <span className="cp-text">{w.text}</span> : null}
              </span>
            ))}
          </div>
        ))}
        {!section.lines.length && (
          <span className="faint" style={{ fontSize: 11 }}>·</span>
        )}
      </div>
    </div>
  );
}
const SheetSection_ = memo(SheetSectionImpl);

function LyricSheetImpl({
  sections,
  selectedChordId,
  activeChordId,
  onSelectChord,
}: {
  sections: SheetSection[];
  /** Chord selected in the timeline (persists while stopped). */
  selectedChordId: string | null;
  /** Chord under the playhead (playback only; changes per chord, not per tick). */
  activeChordId: string | null;
  onSelectChord: (id: string | null) => void;
}) {
  // Playback highlight wins; otherwise the timeline selection.
  const highlightId = activeChordId ?? selectedChordId;
  const rootRef = useRef<HTMLDivElement>(null);

  // Gentle auto-scroll while playing: only when the active chord's mark
  // actually left the viewport (block:'nearest' is a no-op when visible).
  useEffect(() => {
    if (!activeChordId || !rootRef.current) return;
    const el = rootRef.current.querySelector(`[data-cid="${activeChordId}"]`);
    el?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [activeChordId]);

  if (!sections.length) return null;
  return (
    <div ref={rootRef} className="card cmp-sheet">
      <div className="cmp-cap">Lyrics &amp; chords</div>
      {sections.map((sec) => {
        const owns = highlightId != null && sec.chordIds.includes(highlightId);
        return (
          <SheetSection_
            key={sec.id}
            section={sec}
            highlightId={owns ? highlightId : null}
            tinted={owns}
            onSelectChord={onSelectChord}
          />
        );
      })}
    </div>
  );
}

// Memoized: re-renders only when the sheet model, the selection, or the
// ACTIVE CHORD changes — never on playhead ticks.
export const LyricSheet = memo(LyricSheetImpl);
