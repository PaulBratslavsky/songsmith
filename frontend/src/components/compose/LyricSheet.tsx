// The ChordPro lyric SHEET below the Composer timeline (full-song mode
// only) — the readable replacement for the old tick-pinned LyricRow,
// reshaped by "Lyric sheet v2" (COMPOSER-SPEC, user test-drive feedback):
//
//   - ONE section shown at a time: the playing section (follows the
//     playhead), else the selected chord's section, else the first —
//     with compact clickable section CHIPS to browse the others (a chip
//     choice holds until the followed section changes).
//   - A section's lyric lines flow LEFT-TO-RIGHT: each line is an inline
//     chunk that wraps like text, chords still printed in accent above
//     the exact words (the word-level ChordPro model).
//   - Play-along tracking is LINE-based: the hook derives activeLineTick
//     (the line whose [anchor, nextAnchor) range holds the playhead), the
//     sheet tints that line and marks only THAT line's chord occurrence —
//     never name-matched duplicates in other lines/halves.
//
// Two-way sync:
//   timeline → sheet: playback follows the active line; clicking a
//     timeline chord shows its section and marks that exact occurrence
//     (span-id mapping, not name matching).
//   sheet → timeline: clicking a chord selects the matching chord span
//     (same selection state the ChordLane uses).
//
// Perf: the sheet receives `activeChordId` + `activeLineTick`, which the
// playback hook only sets when they CHANGE — never per tick — and the top
// level is memo'd, so the sheet re-renders on chord/line boundaries and a
// tick change renders nothing here at all.

import { memo, useEffect, useMemo, useRef, useState } from 'react';
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
export type SheetLine = {
  words: SheetWord[];
  /** The line's anchor tick (unique per line — see compositionFromSong);
   *  matched against the playback hook's activeLineTick. */
  anchor: number;
};
export type SheetSection = {
  id: string;
  name: string;
  lines: SheetLine[];
  /** Every chord-span id inside the section (maps a chord to its section). */
  chordIds: string[];
};

/**
 * Pure: fold the composition's sections + lyric lines + chord spans into
 * the sheet model. Word-level chord tags (LyricLine.words, threaded from
 * the Lyrics stage's ChordPro) map to this section's chord spans purely
 * POSITIONALLY: occurrence i ↔ the section's i-th span. compositionFromSong
 * (lyric sheet v3) lays a tagged section's spans FROM those placements, so
 * the mapping is 1:1 by construction — no name matching anywhere (name
 * lookups lit "twin" chords when a progression repeated). A line with no
 * word tags falls back to line-level anchoring (the chord at the line's
 * anchor tick, above its first word). Wordless sections (instrumentals)
 * render as a chord-only line.
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

    // Positional in-order mapping: the i-th chord tag in the section IS
    // the section's i-th span (the v3 1:1 invariant). Tags past the
    // section's spans (rows saved before v3 laid spans from placements)
    // stay unlinked — shown but disabled — never a name-matched twin.
    let next = 0;
    const claim = (name?: string): ChordSpan | undefined => {
      if (!name) return undefined;
      const span = spans[next];
      next += 1;
      return span;
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
      return { words, anchor: ln.tick };
    });

    // Instrumental (no lyric lines): one chord-only line, still clickable.
    if (!lines.length && spans.some((s) => s.name)) {
      lines.push({
        anchor: sec.startTick,
        words: spans
          .filter((s) => s.name)
          .map((s) => ({ text: '', chord: s.name, chordId: s.id })),
      });
    }

    return { id: sec.id, name: sec.name, lines, chordIds: spans.map((s) => s.id) };
  });
}

function LyricSheetImpl({
  sections,
  selectedChordId,
  activeChordId,
  activeLineTick,
  onSelectChord,
}: {
  sections: SheetSection[];
  /** Chord selected in the timeline (persists while stopped). */
  selectedChordId: string | null;
  /** Chord under the playhead (playback only; changes per chord, not per tick). */
  activeChordId: string | null;
  /** Anchor tick of the lyric line under the playhead (playback only;
   *  changes per LINE, not per tick). */
  activeLineTick: number | null;
  onSelectChord: (id: string | null) => void;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  // Manual chip browsing — holds until the followed section changes
  // (playhead crosses a section / a different chord is selected).
  const [manualId, setManualId] = useState<string | null>(null);

  const sectionOf = (chordId: string | null) =>
    chordId != null ? (sections.find((s) => s.chordIds.includes(chordId))?.id ?? null) : null;

  // Section to follow (spec order): the selected chord's, else the playing
  // one (active chord's section, else the active line's).
  const followId = useMemo(() => {
    const playing = () =>
      sectionOf(activeChordId) ??
      (activeLineTick != null
        ? (sections.find((s) => s.lines.some((l) => l.anchor === activeLineTick))?.id ?? null)
        : null);
    return sectionOf(selectedChordId) ?? playing();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- sectionOf reads `sections`
  }, [sections, activeChordId, activeLineTick, selectedChordId]);

  // A follow change (chord/line boundary at most) releases the manual chip.
  useEffect(() => {
    setManualId(null);
  }, [followId]);

  const visibleId = manualId ?? followId ?? sections[0]?.id ?? null;
  const visible = sections.find((s) => s.id === visibleId) ?? sections[0];

  // Gentle auto-scroll while playing: keep the active line in view (only
  // scrolls when it actually left the viewport — block:'nearest').
  useEffect(() => {
    if (activeLineTick == null || !rootRef.current) return;
    const el = rootRef.current.querySelector(`[data-line="${activeLineTick}"]`);
    el?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [activeLineTick, visibleId]);

  if (!sections.length || !visible) return null;

  const playing = activeChordId != null || activeLineTick != null;

  return (
    <div ref={rootRef} className="card cmp-sheet">
      <div className="cmp-cap">Lyrics &amp; chords</div>

      {/* Compact section chips — browse the sections one at a time. */}
      <div className="cmp-sheet-chips">
        {sections.map((s) => (
          <button
            key={s.id}
            type="button"
            className={'cmp-sheet-chip' + (s.id === visible.id ? ' active' : '')}
            onClick={() => setManualId(s.id)}
            title={`Show ${s.name}`}
          >
            {s.name}
          </button>
        ))}
      </div>

      {/* The ONE visible section: lines flow left-to-right as inline
          chunks that wrap like text, chords above their exact words. */}
      <div className={'cmp-sheet-section' + (followId === visible.id ? ' active' : '')}>
        <div className="cp-lyrics cmp-sheet-flow">
          {visible.lines.map((line) => {
            const lineActive = activeLineTick != null && line.anchor === activeLineTick;
            return (
              <span
                key={line.anchor}
                data-line={line.anchor}
                className={'cmp-sheet-line' + (lineActive ? ' active' : '')}
              >
                {line.words.map((w, wi) => {
                  // Playing: mark the active chord only inside the ACTIVE
                  // line (span-id match — never name-matched duplicates).
                  // Stopped: mark the selected chord's exact occurrence.
                  const current = playing
                    ? lineActive && w.chordId != null && w.chordId === activeChordId
                    : w.chordId != null && w.chordId === selectedChordId;
                  return (
                    <span key={wi} className="cp-word">
                      {w.chord ? (
                        <button
                          type="button"
                          className={'cp-chord set' + (current ? ' current' : '')}
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
                  );
                })}
              </span>
            );
          })}
          {!visible.lines.length && (
            <span className="faint" style={{ fontSize: 11 }}>
              ·
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

// Memoized: re-renders only when the sheet model, the selection, the
// active CHORD, or the active LINE changes — never on playhead ticks.
export const LyricSheet = memo(LyricSheetImpl);
