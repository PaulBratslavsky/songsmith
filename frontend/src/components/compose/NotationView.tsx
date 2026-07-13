// NotationView — N1: read-only staff-notation rendering of the Composition
// (the "𝄞 Notation" toggle in the Sketchpad; the grid stays the editor).
//
// Two staves per system (melody + bass) with per-staff clef selectors
// (treble/alto/tenor/bass — alto included on purpose), key signature from
// the composition key, 4/4 bars from the tick grid, chord symbols above the
// melody staff, section labels above the system where a section starts, and
// systems that wrap responsively (~4 bars each). All tick→value math lives
// in lib/music/compose/notation.ts; degrees resolve to the same MIDI
// playback sounds (resolveMelodyMidi / resolveBassMidi).
//
// Perf discipline: the component is memo'd and re-renders only when the
// composition, key labels, clefs, container width, or ACTIVE BAR change.
// VexFlow does a full SVG redraw on composition/layout change (acceptable);
// the active-bar playback highlight is a plain absolutely-positioned overlay
// div moved between measure rects captured at draw time — never a VexFlow
// redraw, never per tick.

import { memo, useEffect, useRef, useState } from 'react';
import {
  Accidental,
  BarlineType,
  Beam,
  Dot,
  Formatter,
  Renderer,
  Stave,
  StaveConnector,
  StaveNote,
  StaveTie,
  VexFlow,
  Voice,
} from 'vexflow/bravura';
import type { Composition } from '../../lib/music/compose/types';
import { TICKS_PER_BAR } from '../../lib/music/compose/types';
import {
  resolveBassMidi,
  resolveMelodyMidi,
} from '../../lib/music/compose/playback';
import {
  chordSymbolText,
  decomposeTicks,
  midiSpeller,
  vexKeySpec,
  voiceCells,
  type VoiceCell,
} from '../../lib/music/compose/notation';
import type { DegreeLabel } from '../../lib/music/compose/labels';

type ClefName = 'treble' | 'alto' | 'tenor' | 'bass';
const CLEFS: ClefName[] = ['treble', 'alto', 'tenor', 'bass'];

/** Middle-line rest position per clef, so rests sit centered on any staff. */
const REST_KEY: Record<ClefName, string> = {
  treble: 'b/4',
  alto: 'c/4',
  tenor: 'a/3',
  bass: 'd/3',
};

// Layout constants (px). Text rows (section label + chord symbols) live in
// the headroom VexFlow leaves above a stave's top line (~40px above the
// stave's y), so the system offsets below keep everything inside its band.
const PAD_X = 12;
const STAVE_TOP = 10; // melody stave y within its system band
const BASS_DY = 100; // bass stave y below the melody stave
const SYSTEM_H = 230;
const TOP_PAD = 4;

type MeasureRect = { bar: number; x: number; y: number; w: number; h: number };

/** CSS custom property off the app theme, with a hard fallback. */
function themeColor(el: HTMLElement, name: string, fallback: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim() || fallback;
}

export const NotationView = memo(function NotationView({
  comp,
  labels,
  activeBar,
}: {
  comp: Composition;
  /** Diatonic degree labels for the current key (Sketchpad's memo). */
  labels: Record<number, DegreeLabel>;
  /** Bar index under the playhead (changes on bar boundaries only). */
  activeBar: number | null;
}) {
  const [melodyClef, setMelodyClef] = useState<ClefName>('treble');
  const [bassClef, setBassClef] = useState<ClefName>('bass');
  const [width, setWidth] = useState(0);
  const [rects, setRects] = useState<MeasureRect[]>([]);
  const [fontsReady, setFontsReady] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const hostRef = useRef<HTMLDivElement | null>(null);

  // The bravura entry starts loading its fonts on import; gate the first
  // draw on them so glyphs never render as tofu. Resolves instantly after.
  useEffect(() => {
    let on = true;
    VexFlow.loadFonts('Bravura', 'Academico')
      .then(() => {
        if (on) setFontsReady(true);
      })
      .catch(() => {
        if (on) setFontsReady(true); // draw anyway — better than a blank view
      });
    return () => {
      on = false;
    };
  }, []);

  // Responsive-ish: track the container width (quantized to 8px so window
  // resizes don't thrash the redraw effect).
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = Math.floor(entries[0].contentRect.width / 8) * 8;
      setWidth((prev) => (prev === w ? prev : w));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // ---- full VexFlow redraw: composition / key / clef / width changes only
  useEffect(() => {
    const host = hostRef.current;
    if (!fontsReady || !host || width < 280) return;
    host.innerHTML = '';

    const bars = Math.max(1, Math.ceil(comp.totalTicks / TICKS_PER_BAR));
    const paddedTicks = bars * TICKS_PER_BAR; // last partial bar → rests
    const perSystem = Math.max(2, Math.min(6, Math.floor((width - PAD_X * 2) / 230)));
    const systems = Math.ceil(bars / perSystem);
    const measureW = Math.floor((width - PAD_X * 2) / perSystem);
    const height = TOP_PAD + systems * SYSTEM_H;
    const systemOf = (bar: number) => Math.floor(bar / perSystem);

    const renderer = new Renderer(host, Renderer.Backends.SVG);
    renderer.resize(width, height);
    const ctx = renderer.getContext();

    // Dark theme: engrave in the app's ink on the transparent dark card.
    const ink = themeColor(host, '--ink', '#e8e6e0');
    const inkDim = themeColor(host, '--ink-dim', '#9aa0a6');
    const accent = themeColor(host, '--accent', '#c8ff3d');
    const textFont = getComputedStyle(host).fontFamily || 'monospace';
    ctx.setFillStyle(ink);
    ctx.setStrokeStyle(ink);

    const keySpec = vexKeySpec(comp.key.root, comp.key.mode).spec;
    const spell = midiSpeller(comp);

    // Lane spans → gap-free per-bar cells (rests fill the gaps), then cells
    // → StaveNotes per bar, recording tie chains (within-bar remainders AND
    // across bar lines) against the concrete note objects.
    type TieRef = { a: StaveNote; aBar: number; b: StaveNote; bBar: number };
    const ties: TieRef[] = [];
    const buildBars = (cells: VoiceCell[], clef: ClefName): StaveNote[][] => {
      const byBar: StaveNote[][] = Array.from({ length: bars }, () => []);
      let pending: { note: StaveNote; bar: number } | null = null;
      for (const cell of cells) {
        const rest = cell.midi == null;
        let first: StaveNote | null = null;
        let last: StaveNote | null = null;
        for (const atom of decomposeTicks(cell.ticks)) {
          const note = new StaveNote({
            clef,
            keys: [rest ? REST_KEY[clef] : spell(cell.midi as number)],
            duration: atom.duration + (rest ? 'r' : ''),
            autoStem: !rest,
          });
          if (atom.dotted) Dot.buildAndAttach([note], { all: true });
          byBar[cell.bar].push(note);
          if (!rest) {
            if (last) ties.push({ a: last, aBar: cell.bar, b: note, bBar: cell.bar });
            if (!first) first = note;
            last = note;
          }
        }
        if (pending && first) {
          ties.push({ a: pending.note, aBar: pending.bar, b: first, bBar: cell.bar });
        }
        pending = cell.tieToNext && last ? { note: last, bar: cell.bar } : null;
      }
      return byBar;
    };

    const melodyBars = buildBars(
      voiceCells(comp.melody, paddedTicks, (s) => resolveMelodyMidi(comp, s)),
      melodyClef,
    );
    const bassBars = buildBars(
      voiceCells(comp.bass, paddedTicks, (s) => resolveBassMidi(comp, s)),
      bassClef,
    );

    // Chord symbols keyed by bar (positioned by tick fraction within it)
    // and section labels keyed by their start bar.
    const chordSymbols = comp.chords
      .filter((s) => s.start < paddedTicks)
      .map((s) => ({
        bar: Math.floor(s.start / TICKS_PER_BAR),
        frac: (s.start % TICKS_PER_BAR) / TICKS_PER_BAR,
        text: chordSymbolText(s, labels),
      }))
      .filter((c) => c.text);
    const sectionAtBar = new Map<number, string>();
    for (const s of comp.sections) {
      const bar = Math.floor(s.startTick / TICKS_PER_BAR);
      if (!sectionAtBar.has(bar)) sectionAtBar.set(bar, s.name);
    }

    const nextRects: MeasureRect[] = [];
    for (let bar = 0; bar < bars; bar++) {
      const sys = systemOf(bar);
      const col = bar % perSystem;
      const x = PAD_X + col * measureW;
      const y = TOP_PAD + sys * SYSTEM_H;

      const mel = new Stave(x, y + STAVE_TOP, measureW);
      const bas = new Stave(x, y + STAVE_TOP + BASS_DY, measureW);
      if (col === 0) {
        mel.addClef(melodyClef).addKeySignature(keySpec);
        bas.addClef(bassClef).addKeySignature(keySpec);
        if (sys === 0) {
          mel.addTimeSignature('4/4');
          bas.addTimeSignature('4/4');
        }
      }
      const endType = bar === bars - 1 ? BarlineType.END : BarlineType.SINGLE;
      mel.setEndBarType(endType);
      bas.setEndBarType(endType);
      mel.setContext(ctx).draw();
      bas.setContext(ctx).draw();
      if (col === 0) {
        new StaveConnector(mel, bas).setType('singleLeft').setContext(ctx).draw();
      }

      // Format melody + bass together so the two staves align vertically.
      const melNotes = melodyBars[bar];
      const basNotes = bassBars[bar];
      const vMel = new Voice({ numBeats: 4, beatValue: 4 })
        .setMode(Voice.Mode.SOFT)
        .addTickables(melNotes);
      const vBas = new Voice({ numBeats: 4, beatValue: 4 })
        .setMode(Voice.Mode.SOFT)
        .addTickables(basNotes);
      // Key-signature-aware accidentals, tracked per staff per measure.
      Accidental.applyAccidentals([vMel], keySpec);
      Accidental.applyAccidentals([vBas], keySpec);
      const beams = [...Beam.generateBeams(melNotes), ...Beam.generateBeams(basNotes)];
      const noteW = measureW - (mel.getNoteStartX() - mel.getX()) - 14;
      new Formatter()
        .joinVoices([vMel])
        .joinVoices([vBas])
        .format([vMel, vBas], Math.max(40, noteW));
      vMel.draw(ctx, mel);
      vBas.draw(ctx, bas);
      for (const beam of beams) beam.setContext(ctx).draw();

      // Chord symbols + section label in the headroom above the melody staff.
      const topLineY = mel.getYForLine(0);
      ctx.save();
      ctx.setFont(textFont, 11, 'bold');
      for (const c of chordSymbols) {
        if (c.bar !== bar) continue;
        const noteX0 = mel.getNoteStartX();
        const cx = noteX0 + c.frac * (mel.getX() + mel.getWidth() - noteX0);
        ctx.setFillStyle(inkDim);
        ctx.fillText(c.text, cx, topLineY - 12);
      }
      const section = sectionAtBar.get(bar);
      if (section) {
        ctx.setFillStyle(accent);
        ctx.setFont(textFont, 10, 'bold');
        ctx.fillText(section.toUpperCase(), mel.getX() + 2, topLineY - 28);
      }
      ctx.restore();
      ctx.setFillStyle(ink);
      ctx.setStrokeStyle(ink);

      const rectY = topLineY - 34;
      nextRects.push({
        bar,
        x,
        y: rectY,
        w: measureW,
        h: bas.getYForLine(4) + 12 - rectY,
      });
    }

    // Ties last, once every note has a stave/x: same-system ties draw whole,
    // system-crossing ties draw as the conventional two half-ties.
    for (const t of ties) {
      if (systemOf(t.aBar) === systemOf(t.bBar)) {
        new StaveTie({
          firstNote: t.a,
          lastNote: t.b,
          firstIndexes: [0],
          lastIndexes: [0],
        })
          .setContext(ctx)
          .draw();
      } else {
        new StaveTie({ firstNote: t.a, firstIndexes: [0], lastIndexes: [0] })
          .setContext(ctx)
          .draw();
        new StaveTie({ lastNote: t.b, firstIndexes: [0], lastIndexes: [0] })
          .setContext(ctx)
          .draw();
      }
    }

    setRects(nextRects);
  }, [fontsReady, comp, labels, melodyClef, bassClef, width]);

  const active =
    activeBar != null ? rects.find((r) => r.bar === activeBar) : undefined;

  return (
    <div className="card">
      <div className="row" style={{ alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
        {(
          [
            ['Melody clef', melodyClef, setMelodyClef],
            ['Bass clef', bassClef, setBassClef],
          ] as Array<[string, ClefName, (c: ClefName) => void]>
        ).map(([label, value, set]) => (
          <label
            key={label}
            className="row"
            style={{ margin: 0, alignItems: 'center', gap: 6, textTransform: 'none' }}
          >
            <span className="cmp-cap">{label}</span>
            <select
              value={value}
              onChange={(e) => set(e.target.value as ClefName)}
              style={{ width: 'auto', padding: '3px 6px', fontSize: 12 }}
            >
              {CLEFS.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          </label>
        ))}
        <span className="faint" style={{ fontSize: 11, marginLeft: 'auto' }}>
          read-only — edit in ▦ Grid view
        </span>
      </div>
      <div ref={wrapRef} style={{ position: 'relative', marginTop: 8 }}>
        {active && (
          <div
            style={{
              position: 'absolute',
              left: active.x,
              top: active.y,
              width: active.w,
              height: active.h,
              background: 'var(--accent)',
              opacity: 0.09,
              borderRadius: 2,
              pointerEvents: 'none',
            }}
          />
        )}
        <div ref={hostRef} />
      </div>
    </div>
  );
});
