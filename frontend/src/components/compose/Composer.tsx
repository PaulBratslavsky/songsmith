// Progression Composer (the NEW visual builder) — top-level. Owns the
// Composition and assembles the Hookpad-style stacked layout: transport,
// chord palette, then a shared 8-bar timeline with a melody piano-roll,
// the draggable chord lane, and a bass piano-roll. Span/note mutations go
// through the pure helpers in spans.ts; this component only wires state to
// UI.
//
// Time is in ticks (sixteenth resolution). A duration picker sets the
// length of newly-placed chords and notes; everything is draggable and
// resizable afterwards.
//
// NOTE: this is distinct from the chord-section editor in
// components/Composer.tsx and the /builder Chord Builder. Compositions
// persist in the libSQL `composition` table (Phase 3): Save serializes the
// reducer state through CompositionSchema; the library panel reopens rows
// via parseStoredComposition + the load action (which reidentifies spans).

import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../ipc/api';
import type { CompositionMeta } from '../../ipc/generated';
import { PITCH_CLASSES, type PitchClass } from '../../lib/music/types';
import {
  DURATIONS,
  emptyComposition,
  type Composition,
  type Degree,
  type KeyMode,
} from '../../lib/music/compose/types';
import { LABEL_W, BAR_MIN_PX } from './laneLayout';
import { CompositionSchema, parseStoredComposition } from '../../lib/music/compose/schema';
import { useCompositionState } from '../../lib/music/compose/useCompositionState';
import { useCompositionPlayback } from '../../lib/music/compose/useCompositionPlayback';
import { synth } from '../../music/synth';
import {
  keyToScaleSelection,
  resolveMelodyMidi,
  resolveBassMidi,
} from '../../lib/music/compose/playback';
import { getScalePitchClasses } from '../../lib/music/theory/scales';
import { getDiatonicChords } from '../../lib/music/theory/diatonic';
import {
  chordToneDegrees,
  degreeLabel,
  type DegreeLabel,
} from '../../lib/music/compose/labels';
import { degreeColor, hexToRgba } from '../../lib/music/compose/colors';
import type { ChordToneHighlight } from './chordHighlight';
import { BeatRuler } from './BeatRuler';
import { ChordPalette } from './ChordPalette';
import { ChordLane } from './ChordLane';
import { NoteLane } from './NoteLane';
import { SectionBand } from './SectionBand';
import { LyricSheet, buildSheetModel } from './LyricSheet';
import { ExportDialog } from './ExportDialog';

const MELODY_COLOR = '#2563eb';
const BASS_COLOR = '#9333ea';

/** A small demo sketch so the surface isn't blank on first open. */
function demoComposition(id: string): Composition {
  const c = emptyComposition(id, 'Sketch', 'C', 'major');
  c.chords = [
    { id: 'd-c1', degree: 1, seventh: false, start: 0, length: 16 },
    { id: 'd-c2', degree: 5, seventh: false, start: 16, length: 16 },
    { id: 'd-c3', degree: 6, seventh: false, start: 32, length: 16 },
    { id: 'd-c4', degree: 4, seventh: false, start: 48, length: 16 },
  ];
  c.melody = [
    { id: 'd-m1', degree: 1, octave: 0, start: 0, length: 4 },
    { id: 'd-m2', degree: 3, octave: 0, start: 4, length: 4 },
    { id: 'd-m3', degree: 5, octave: 0, start: 8, length: 8 },
    { id: 'd-m4', degree: 5, octave: 0, start: 16, length: 4 },
    { id: 'd-m5', degree: 4, octave: 0, start: 20, length: 4 },
    { id: 'd-m6', degree: 2, octave: 0, start: 24, length: 8 },
  ];
  c.bass = [
    { id: 'd-b1', degree: 1, octave: 0, start: 0, length: 16 },
    { id: 'd-b2', degree: 5, octave: 0, start: 16, length: 16 },
    { id: 'd-b3', degree: 6, octave: 0, start: 32, length: 16 },
    { id: 'd-b4', degree: 4, octave: 0, start: 48, length: 16 },
  ];
  return c;
}

export function Composer({
  initialRoot = 'C',
  initial,
  songId = null,
}: {
  initialRoot?: PitchClass;
  /** A full-song (or any) Composition to seed/load — full-song import.
   *  When absent, the blank-sketch demo is used (unchanged behavior). */
  initial?: Composition | null;
  /** Source song id when `initial` is a full-song import — saved rows
   *  remember it (`song_id`), so the library can badge them. */
  songId?: string | null;
}) {
  // Composition + edit state (comp, cursor, selected) live in the reducer;
  // only ephemeral UI stays local here. Seeded with the supplied initial
  // composition (full-song import) or a small demo sketch.
  const { comp, cursor, selected, actions } = useCompositionState(
    initialRoot,
    initial ? () => initial : demoComposition,
  );
  // Selection scoped PER LANE, so selecting in one lane only re-renders
  // that lane (the other memo'd lanes see an unchanged `null`).
  const melodySelId = selected?.kind === 'melody' ? selected.id : null;
  const chordSelId = selected?.kind === 'chord' ? selected.id : null;
  const bassSelId = selected?.kind === 'bass' ? selected.id : null;

  // ---- persistence (Phase 3): saved-row identity + dirty tracking ----
  // `savedRowId` is the libSQL row this editor is bound to (null = never
  // saved → the next Save inserts and ADOPTS the minted row id).
  // `baseline` is the serialized comp as of the last save/load/new-blank;
  // dirty = current serialization differs. `baselinePending` re-snapshots
  // on the render AFTER a load/reset lands (the load action mints fresh
  // span ids, so the snapshot can only be taken from the reducer output).
  const [savedRowId, setSavedRowId] = useState<string | null>(null);
  const [linkedSongId, setLinkedSongId] = useState<string | null>(songId ?? null);
  const [baseline, setBaseline] = useState<string | null>(null);
  const baselinePending = useRef(true); // snapshot the very first comp too
  const serialized = useMemo(() => JSON.stringify(comp), [comp]);
  useEffect(() => {
    if (baselinePending.current) {
      baselinePending.current = false;
      setBaseline(serialized);
    }
  }, [serialized]);
  const dirty = baseline != null && serialized !== baseline;

  const [libOpen, setLibOpen] = useState(false);
  const [library, setLibrary] = useState<CompositionMeta[]>([]);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  // ⤴ Export (composition → song): update the linked song or create a new one
  const [exportOpen, setExportOpen] = useState(false);

  // If the imported composition changes (navigating to a different song,
  // or the song's chords/lyrics load in), load it into the editor.
  const loadedId = useRef<string | null>(initial?.id ?? null);
  useEffect(() => {
    if (initial && initial.id !== loadedId.current) {
      loadedId.current = initial.id;
      actions.load(initial);
      // a fresh import is a new, unsaved composition bound to its song
      setSavedRowId(null);
      setLinkedSongId(songId ?? null);
      baselinePending.current = true;
      setSaveMsg(null);
    }
  }, [initial, songId, actions]);

  const [muted, setMuted] = useState(false);
  const [durTicks, setDurTicks] = useState(4); // default 1/4 note
  const [loop, setLoop] = useState(true);
  // Sticky placement mode: newly-dropped chords are sevenths while on.
  const [seventhMode, setSeventhMode] = useState(false);

  const { isPlaying, currentStep, activeChordId, toggle, stop } = useCompositionPlayback(
    comp,
    { loop },
  );

  useEffect(() => {
    synth.setMuted(muted);
  }, [muted]);

  // Note placement carries the duration-picker length, read via a ref so
  // the stable handler bundles below don't churn when the duration changes.
  const durRef = useRef(durTicks);
  durRef.current = durTicks;

  // Key-derived data (changes only when the key changes) so the memoized
  // lanes don't re-render on note/chord edits or playback ticks.
  const scaleSel = useMemo(
    () => keyToScaleSelection(comp),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- key-only
    [comp.key.root, comp.key.mode],
  );
  const pcs = useMemo(() => getScalePitchClasses(scaleSel), [scaleSel]);
  const labels = useMemo(() => {
    const m: Record<number, DegreeLabel> = {};
    for (const c of getDiatonicChords(scaleSel)) m[c.degree] = degreeLabel(c);
    return m;
  }, [scaleSel]);

  // Latest preview fns (depend on key) read through a ref so the stable
  // handler bundles don't change identity when the composition edits.
  const previewRef = useRef<{ melody: (d: Degree) => void; bass: (d: Degree) => void }>({
    melody: () => {},
    bass: () => {},
  });
  previewRef.current = {
    melody: (d) => {
      const midi = resolveMelodyMidi(comp, { degree: d, octave: 0 });
      if (midi != null) synth.playNote(midi, 260, 'piano');
    },
    bass: (d) => {
      const midi = resolveBassMidi(comp, { degree: d, octave: 0 });
      if (midi != null) synth.playNote(midi, 260, 'bass');
    },
  };

  // Stable handler bundles (actions is stable; dur/preview via refs) so
  // the memoized lanes only re-render when their own data changes.
  const melodyHandlers = useMemo(
    () => ({
      onPlace: (degree: Degree, tick: number) => actions.placeNote('melody', degree, tick, durRef.current),
      onSelect: (id: string | null) => (id ? actions.select('melody', id) : actions.deselect()),
      onMove: (id: string, s: number, degree: Degree) => actions.moveNote('melody', id, s, degree),
      onResize: (id: string, l: number) => actions.resizeNote('melody', id, l),
      onRemove: (id: string) => actions.removeNote('melody', id),
      previewNote: (d: Degree) => previewRef.current.melody(d),
    }),
    [actions],
  );
  const bassHandlers = useMemo(
    () => ({
      onPlace: (degree: Degree, tick: number) => actions.placeNote('bass', degree, tick, durRef.current),
      onSelect: (id: string | null) => (id ? actions.select('bass', id) : actions.deselect()),
      onMove: (id: string, s: number, degree: Degree) => actions.moveNote('bass', id, s, degree),
      onResize: (id: string, l: number) => actions.resizeNote('bass', id, l),
      onRemove: (id: string) => actions.removeNote('bass', id),
      previewNote: (d: Degree) => previewRef.current.bass(d),
    }),
    [actions],
  );
  const chordHandlers = useMemo(
    () => ({
      onSelect: (id: string | null) => (id ? actions.select('chord', id) : actions.deselect()),
      onSetCursor: actions.selectBar,
      onMove: actions.moveChord,
      onResize: actions.resizeChord,
      onRemove: actions.removeChord,
    }),
    [actions],
  );

  // ---- keyboard: delete selected, escape to deselect ----
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing =
        el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT');
      if (e.key === 'Escape') {
        actions.deselect();
        return;
      }
      if ((e.key === 'Delete' || e.key === 'Backspace') && selected && !typing) {
        e.preventDefault();
        if (selected.kind === 'chord') actions.removeChord(selected.id);
        else actions.removeNote(selected.kind, selected.id);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [selected, actions]);

  // ---- selected-chord tone highlight in the melody grid ----
  const selectedChord = chordSelId
    ? comp.chords.find((s) => s.id === chordSelId)
    : undefined;
  const melodyHighlight = useMemo<ChordToneHighlight | null>(
    () =>
      selectedChord
        ? {
            degrees: new Set(
              chordToneDegrees(selectedChord.degree, selectedChord.seventh),
            ),
            start: selectedChord.start,
            length: selectedChord.length,
            color: hexToRgba(degreeColor(selectedChord.degree), 0.28),
          }
        : null,
    [selectedChord],
  );

  // ---- lyric sheet model (full-song mode only; empty for blank sketches).
  // Rebuilt when sections/lyrics/chords change — NOT on playhead ticks.
  const sheetSections = useMemo(
    () => buildSheetModel(comp.sections, comp.lyrics, comp.chords),
    [comp.sections, comp.lyrics, comp.chords],
  );

  const clearAll = () => {
    stop();
    actions.clearAll();
  };

  // ---- persistence handlers (save / library / open / delete / new) ----
  const refreshLibrary = async () => {
    try {
      setLibrary(await api.listCompositions());
    } catch (e) {
      setSaveMsg(String(e));
    }
  };

  const doSave = async () => {
    // one serializer/validator: the same CompositionSchema the load seam
    // trusts. An empty name would fail zod's min(1), so default it.
    const name = comp.name.trim() || 'Untitled';
    const candidate = { ...comp, name };
    const parsed = CompositionSchema.safeParse(candidate);
    if (!parsed.success) {
      setSaveMsg(`can't save — ${parsed.error.issues[0]?.message ?? 'invalid composition'}`);
      return;
    }
    try {
      const row = await api.saveComposition(savedRowId, name, linkedSongId, JSON.stringify(parsed.data));
      setSavedRowId(row.id); // adopt the row id — later saves update in place
      if (name !== comp.name) actions.setName(name);
      setBaseline(JSON.stringify(candidate));
      setSaveMsg(null);
      if (libOpen) void refreshLibrary();
    } catch (e) {
      setSaveMsg(String(e));
    }
  };

  const openComposition = async (id: string) => {
    if (dirty && !window.confirm('Discard unsaved changes and open this composition?')) return;
    try {
      const row = await api.getComposition(id);
      if (!row) {
        setSaveMsg('composition not found — it may have been deleted');
        void refreshLibrary();
        return;
      }
      let blob: unknown = null;
      try {
        blob = JSON.parse(row.data);
      } catch {}
      // the designed load seam: validate/migrate the stored blob, then the
      // load action reidentifies span/section ids so they can't collide
      const stored = blob == null ? null : parseStoredComposition(blob);
      if (!stored) {
        setSaveMsg(`"${row.name}" couldn't be read — leaving the editor as is`);
        return;
      }
      stop();
      actions.load(stored);
      setSavedRowId(row.id);
      setLinkedSongId(row.song_id);
      baselinePending.current = true; // snapshot the loaded comp next render
      setLibOpen(false);
      setSaveMsg(null);
    } catch (e) {
      setSaveMsg(String(e));
    }
  };

  const removeComposition = async (id: string, name: string) => {
    if (!window.confirm(`Delete "${name}" from the library? This can't be undone.`)) return;
    try {
      await api.deleteComposition(id);
      if (id === savedRowId) setSavedRowId(null); // editor keeps the comp, now unsaved
      void refreshLibrary();
    } catch (e) {
      setSaveMsg(String(e));
    }
  };

  const newBlank = () => {
    if (dirty && !window.confirm('Discard unsaved changes and start a new blank sketch?')) return;
    stop();
    actions.reset(comp.key.root, comp.key.mode);
    setSavedRowId(null);
    setLinkedSongId(null);
    baselinePending.current = true;
    setSaveMsg(null);
  };

  const toggleLibrary = () => {
    setLibOpen((o) => {
      if (!o) void refreshLibrary();
      return !o;
    });
  };

  return (
    <div className="col" style={{ gap: 14 }}>
      {/* Transport */}
      <div className="card cmp-transport">
        <button
          type="button"
          className="primary"
          onClick={toggle}
          style={{ minWidth: 84 }}
        >
          {isPlaying ? '■ Stop' : '▶ Play'}
        </button>

        <div className="row" style={{ alignItems: 'center', gap: 6 }}>
          <span className="cmp-cap">Key</span>
          <div className="row" style={{ flexWrap: 'wrap', gap: 3 }}>
            {PITCH_CLASSES.map((pc) => (
              <button
                key={pc}
                type="button"
                className={'sm' + (comp.key.root === pc ? ' primary' : '')}
                onClick={() => actions.setKeyRoot(pc)}
              >
                {pc}
              </button>
            ))}
          </div>
        </div>

        <div className="row" style={{ gap: 3 }}>
          {(['major', 'minor'] as KeyMode[]).map((m) => (
            <button
              key={m}
              type="button"
              className={'sm' + (comp.key.mode === m ? ' primary' : '')}
              onClick={() => actions.setKeyMode(m)}
              style={{ textTransform: 'capitalize' }}
            >
              {m}
            </button>
          ))}
        </div>

        <label className="row" style={{ margin: 0, alignItems: 'center', gap: 8, textTransform: 'none' }}>
          <span className="cmp-cap">Tempo</span>
          <input
            type="range"
            min={60}
            max={180}
            value={comp.bpm}
            onChange={(e) => actions.setBpm(Number(e.target.value))}
            style={{ width: 96, padding: 0 }}
          />
          <span style={{ width: 56, fontVariantNumeric: 'tabular-nums', color: 'var(--ink-dim)' }}>
            {comp.bpm} BPM
          </span>
        </label>

        <div className="row" style={{ alignItems: 'center', gap: 4 }}>
          <span className="cmp-cap">Note</span>
          <div className="row" style={{ gap: 3 }}>
            {DURATIONS.map((d) => (
              <button
                key={d.ticks}
                type="button"
                className={'sm' + (durTicks === d.ticks ? ' primary' : '')}
                onClick={() => setDurTicks(d.ticks)}
              >
                {d.label}
              </button>
            ))}
          </div>
        </div>

        <button type="button" className="sm" onClick={() => setMuted((m) => !m)}>
          {muted ? '🔇 Muted' : '🔊 Sound'}
        </button>

        <button
          type="button"
          className={'sm' + (loop ? ' primary' : '')}
          onClick={() => setLoop((l) => !l)}
          title="Loop playback"
        >
          ↻ Loop {loop ? 'on' : 'off'}
        </button>

        <button type="button" className="sm ghost" onClick={clearAll} style={{ marginLeft: 'auto' }}>
          Clear
        </button>
      </div>

      {/* Name + persistence: Save (insert-or-update), library, New blank */}
      <div className="row" style={{ flexWrap: 'wrap', gap: 8, alignItems: 'center' }}>
        <input
          type="text"
          value={comp.name}
          onChange={(e) => actions.setName(e.target.value)}
          placeholder="Composition name"
          style={{ width: 200 }}
        />
        <button
          type="button"
          className={'sm' + (dirty || !savedRowId ? ' primary' : '')}
          onClick={() => void doSave()}
          title={savedRowId ? 'Update the saved composition' : 'Save this composition to the library'}
        >
          💾 Save
        </button>
        <button
          type="button"
          className={'sm' + (libOpen ? ' primary' : '')}
          onClick={toggleLibrary}
          title="Open a saved composition"
        >
          📂 Open
        </button>
        <button
          type="button"
          className="sm"
          onClick={() => setExportOpen(true)}
          title="Export this composition into a song — update the linked song's Chords + Structure stages, or create a new song from it (degrees resolve to absolute chords in the composition's key)"
        >
          ⤴ Export
        </button>
        <button type="button" className="sm ghost" onClick={newBlank}>
          New blank
        </button>
        <span className="faint" style={{ fontSize: 11 }}>
          {saveMsg ?? (savedRowId ? (dirty ? 'unsaved changes' : 'saved') : 'not saved yet')}
        </span>
      </div>

      {/* Saved-compositions library: open a row (confirm if dirty) or delete */}
      {libOpen && (
        <div className="card cmp-library">
          <div className="row" style={{ alignItems: 'baseline', gap: 8 }}>
            <b style={{ fontSize: 12 }}>Saved compositions</b>
            <span className="faint" style={{ fontSize: 11 }}>
              stored locally — newest first
            </span>
          </div>
          {library.length === 0 ? (
            <p className="faint" style={{ margin: '8px 0 2px', fontSize: 12 }}>
              Nothing saved yet — hit 💾 Save to keep the current sketch.
            </p>
          ) : (
            library.map((c) => (
              <div
                key={c.id}
                className="row"
                style={{
                  alignItems: 'center',
                  gap: 8,
                  padding: '5px 0',
                  borderTop: '1px solid var(--line)',
                  marginTop: 5,
                }}
              >
                <button
                  type="button"
                  className="sm ghost"
                  onClick={() => void openComposition(c.id)}
                  title="Open this composition"
                  style={{ flex: 1, textAlign: 'left', fontWeight: c.id === savedRowId ? 700 : 400 }}
                >
                  {c.name}
                </button>
                {c.song_id && (
                  <span className="badge published" title={`Imported from song ${c.song_id}`}>
                    ♪ song
                  </span>
                )}
                <span className="faint" style={{ fontSize: 11, fontVariantNumeric: 'tabular-nums' }}>
                  {new Date(c.updated_at).toLocaleString()}
                </span>
                <button
                  type="button"
                  className="sm ghost"
                  aria-label="Delete composition"
                  title="Delete this composition"
                  onClick={() => void removeComposition(c.id, c.name)}
                >
                  ×
                </button>
              </div>
            ))
          )}
        </div>
      )}

      {/* Palette */}
      <div className="row" style={{ flexWrap: 'wrap', alignItems: 'center', gap: 12 }}>
        <ChordPalette
          comp={comp}
          seventh={selectedChord ? selectedChord.seventh : seventhMode}
          onPick={(degree) => actions.pickDegree(degree, seventhMode)}
        />
        <button
          type="button"
          className={
            'sm' + ((selectedChord ? selectedChord.seventh : seventhMode) ? ' primary' : '')
          }
          onClick={() => {
            if (selectedChord) {
              actions.setChordSeventh(selectedChord.id, !selectedChord.seventh);
            } else {
              setSeventhMode((m) => !m);
            }
          }}
          aria-pressed={selectedChord ? selectedChord.seventh : seventhMode}
          title={
            selectedChord
              ? 'Toggle the selected chord between triad and seventh'
              : 'Place new chords as four-note sevenths'
          }
        >
          7th
        </button>
      </div>
      <p className="faint" style={{ marginTop: -6, fontSize: 11 }}>
        {selectedChord
          ? 'Chord selected — pick a palette chip to change it, "7th" to toggle the seventh, drag its body to move, the right edge to extend, or × to remove.'
          : `Click a beat in the chord lane to set where the next chord lands, then a palette chip. "7th" ${seventhMode ? 'is on — new chords are sevenths.' : 'makes new chords sevenths.'} Click melody/bass cells to add notes at the chosen duration.`}
      </p>

      {/* Timeline — horizontally scrollable; long (full-song) compositions
          get a wide track so sections/chords stay legible. */}
      <div className="card" style={{ overflowX: 'auto' }}>
        <div style={{ position: 'relative', minWidth: Math.max(820, comp.bars * BAR_MIN_PX) }}>
          {/* Moving playhead — spans all lanes at the current tick. The
              track starts after the LABEL_W gutter, so offset by it. */}
          {currentStep != null && (
            <div
              style={{
                pointerEvents: 'none',
                position: 'absolute',
                bottom: 0,
                top: 16,
                zIndex: 10,
                width: 2,
                background: 'var(--accent)',
                opacity: 0.7,
                left: `calc(${LABEL_W} + (100% - ${LABEL_W}) * ${(currentStep + 0.5) / comp.totalTicks})`,
              }}
            />
          )}
          <SectionBand sections={comp.sections} totalTicks={comp.totalTicks} />
          <BeatRuler bars={comp.bars} totalTicks={comp.totalTicks} />
          <div style={{ marginTop: 4 }}>
            <NoteLane
              lane="melody"
              notes={comp.melody}
              pcs={pcs}
              color={MELODY_COLOR}
              totalTicks={comp.totalTicks}
              highlight={melodyHighlight}
              selectedId={melodySelId}
              {...melodyHandlers}
            />
          </div>
          <div style={{ margin: '6px 0' }}>
            <ChordLane
              chords={comp.chords}
              labels={labels}
              totalTicks={comp.totalTicks}
              selectedId={chordSelId}
              cursor={selected ? -1 : cursor}
              {...chordHandlers}
            />
          </div>
          <NoteLane
            lane="bass"
            notes={comp.bass}
            pcs={pcs}
            color={BASS_COLOR}
            totalTicks={comp.totalTicks}
            selectedId={bassSelId}
            {...bassHandlers}
          />
          <div className="row" style={{ marginTop: 8, gap: 16, fontSize: 10, color: 'var(--ink-faint)' }}>
            <span className="row" style={{ gap: 4, alignItems: 'center' }}>
              <span style={{ display: 'inline-block', height: 8, width: 8, borderRadius: 2, backgroundColor: MELODY_COLOR }} />
              Melody
            </span>
            <span className="row" style={{ gap: 4, alignItems: 'center' }}>
              <span style={{ display: 'inline-block', height: 8, width: 8, borderRadius: 2, backgroundColor: BASS_COLOR }} />
              Bass
            </span>
          </div>
        </div>
      </div>

      {/* ChordPro lyric sheet — the readable view below the timeline
          (full-song mode only; blank sketches have no sections → no sheet).
          Highlight follows the selected chord / the playhead's chord. */}
      <LyricSheet
        sections={sheetSections}
        selectedChordId={chordSelId}
        activeChordId={activeChordId}
        onSelectChord={chordHandlers.onSelect}
      />

      {/* ⤴ Export dialog — composition → song (update linked / create new) */}
      {exportOpen && (
        <ExportDialog comp={comp} songId={linkedSongId} onClose={() => setExportOpen(false)} />
      )}
    </div>
  );
}
