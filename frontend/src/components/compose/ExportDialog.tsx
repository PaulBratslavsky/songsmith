// ⤴ Export dialog — pushes the Composer's chords (+ structure) back into a
// song (COMPOSER-SPEC.md #2/#3, the final leg of the bidirectional loop).
// The degree→absolute mapping happens HERE in the frontend via the pure
// resolveCompositionSections (lib/music/compose/compositionToSong.ts); the
// backend receives fully resolved sections and stays theory-free.
//
// Two destinations:
//   - Update the linked song (when the composition came from one): overwrites
//     the Chords stage and back-fills Structure like a lyric paste. 🔒 frozen
//     sections are listed up front and SKIPPED (kept as they are) — never
//     silently overwritten.
//   - Create a new song (any composition, blank sketches included): mirrors
//     the create flow's preset/title inputs; key/bpm come from the
//     composition; lyrics stay empty.

import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from '@tanstack/react-router';
import { useQueryClient } from '@tanstack/react-query';
import { api } from '../../ipc/api';
import type { StylePreset } from '../../ipc/generated';
import type { Composition } from '../../lib/music/compose/types';
import { resolveCompositionSections } from '../../lib/music/compose/compositionToSong';

export function ExportDialog({
  comp,
  songId,
  onClose,
}: {
  comp: Composition;
  /** The linked source song (full-song imports) — null for blank sketches. */
  songId: string | null;
  onClose: () => void;
}) {
  const nav = useNavigate();
  const qc = useQueryClient();

  // Degree→name resolution (pure; `name` wins, else the diatonic chord for
  // the degree in the composition's key).
  const sections = useMemo(() => resolveCompositionSections(comp), [comp]);
  const chordCount = sections.reduce((n, s) => n + s.chords.length, 0);

  const [mode, setMode] = useState<'update' | 'create'>(songId ? 'update' : 'create');
  const [title, setTitle] = useState(comp.name.trim() || 'Untitled song');
  const [presets, setPresets] = useState<StylePreset[]>([]);
  const [presetId, setPresetId] = useState('');
  const [songTitle, setSongTitle] = useState<string | null>(null);
  const [frozen, setFrozen] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Presets for the create form + the linked song's title and its 🔒 frozen
  // chord/structure section labels (surfaced before the user confirms).
  useEffect(() => {
    let alive = true;
    api.listStylePresets().then((p) => alive && setPresets(p)).catch(() => {});
    if (!songId) return () => { alive = false; };
    (async () => {
      try {
        const detail = await api.getSong(songId);
        if (!alive || !detail) return;
        setSongTitle(detail.song.title || 'Untitled song');
        const locked = new Set<string>();
        for (const t of ['chords', 'structure'] as const) {
          const stage = detail.stages.find((s) => s.type === t);
          if (!stage) continue;
          const sd = await api.getStage(stage.id);
          try {
            const data = JSON.parse(sd?.artifact?.content ?? '')?.data;
            for (const sec of data?.sections ?? []) {
              if (sec?.frozen) locked.add(sec.label || sec.type || 'Section');
            }
          } catch {}
        }
        if (alive) setFrozen([...locked]);
      } catch {}
    })();
    return () => { alive = false; };
  }, [songId]);

  const goToSong = (id: string) => {
    // the export just rewrote stage artifacts — drop the cached queries so
    // the workspace (and any composer rebuild) reads the new revisions
    qc.invalidateQueries({ queryKey: ['song'] });
    qc.invalidateQueries({ queryKey: ['songs'] });
    qc.invalidateQueries({ queryKey: ['stage'] });
    onClose();
    nav({ to: '/song/$id', params: { id } });
  };

  const doExport = async () => {
    setBusy(true);
    setError(null);
    try {
      const json = JSON.stringify(sections);
      if (mode === 'update' && songId) {
        await api.exportCompositionToSong(songId, json);
        goToSong(songId);
      } else {
        const song = await api.createSongFromComposition(
          presetId || presets[0]?.id || '',
          title.trim() || comp.name.trim() || 'Untitled song',
          comp.key.root,
          comp.key.mode,
          comp.bpm,
          json,
        );
        goToSong(song.id);
      }
    } catch (e: any) {
      setError(String(e?.message ?? e));
      setBusy(false);
    }
  };

  const noPresets = presets.length === 0;
  const createDisabled = mode === 'create' && noPresets;

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ width: 560, maxWidth: '92vw' }}>
        <h2>⤴ Export to song</h2>
        <p className="muted">
          The composition's chords become a real song's <b>Chords</b> + <b>Structure</b> stages —
          degrees resolve to absolute chords in {comp.key.root} {comp.key.mode}.
        </p>

        {/* What will be exported (resolved, absolute names) */}
        <div style={{ border: '1px solid var(--line)', borderRadius: 2, padding: 8, maxHeight: 160, overflowY: 'auto', marginBottom: 10 }}>
          <div className="row" style={{ gap: 6, marginBottom: 6, flexWrap: 'wrap' }}>
            <span className="badge">{sections.length} section{sections.length === 1 ? '' : 's'}</span>
            <span className="badge">{chordCount} chord{chordCount === 1 ? '' : 's'}</span>
            <span className="faint" style={{ fontSize: 11 }}>{comp.key.root} {comp.key.mode} · {comp.bpm} BPM</span>
          </div>
          {sections.map((s, i) => (
            <div key={i} style={{ fontFamily: 'var(--mono)', fontSize: 11, padding: '1px 0' }}>
              <b>{s.label}</b> ({s.bars} bars): {s.chords.map((c) => c.name).join(' ') || '—'}
            </div>
          ))}
        </div>
        {chordCount === 0 && (
          <div className="banner warn" style={{ marginBottom: 10 }}>
            No chords on the timeline yet — the song's sections would export empty.
          </div>
        )}

        {/* Destination */}
        {songId && (
          <div className="col" style={{ gap: 6, marginBottom: 10 }}>
            <label className="row" style={{ gap: 8, alignItems: 'center', margin: 0, textTransform: 'none', cursor: 'pointer' }}>
              <input type="radio" checked={mode === 'update'} onChange={() => setMode('update')} />
              <span>Update <b>“{songTitle ?? 'the linked song'}”</b> — overwrite its Chords &amp; Structure stages</span>
            </label>
            {mode === 'update' && frozen.length > 0 && (
              <div className="banner warn" style={{ marginLeft: 22 }}>
                🔒 Locked sections are kept as they are (skipped, never overwritten):{' '}
                <b>{frozen.join(', ')}</b>. Unlock them in the song to export over them.
              </div>
            )}
            <label className="row" style={{ gap: 8, alignItems: 'center', margin: 0, textTransform: 'none', cursor: 'pointer' }}>
              <input type="radio" checked={mode === 'create'} onChange={() => setMode('create')} />
              <span>Create a new song from this composition</span>
            </label>
          </div>
        )}

        {mode === 'create' && (
          <div className="row" style={{ gap: 10, flexWrap: 'wrap', marginBottom: 4 }}>
            <div style={{ flex: 1, minWidth: 180 }}>
              <label>Style preset</label>
              <select value={presetId || presets[0]?.id || ''} onChange={(e) => setPresetId(e.target.value)} style={{ width: '100%' }}>
                {presets.map((p) => (<option key={p.id} value={p.id}>{p.name}</option>))}
              </select>
            </div>
            <div style={{ flex: 1, minWidth: 180 }}>
              <label>Working title</label>
              <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g. Taillights" style={{ width: '100%' }} />
            </div>
          </div>
        )}
        {mode === 'create' && (
          <p className="faint" style={{ fontSize: 11, margin: '4px 0 0' }}>
            Structure + Chords land filled in ({comp.key.root} {comp.key.mode}, {comp.bpm} BPM); lyrics stay
            empty — melody &amp; bass live in the saved composition.
            {noPresets && ' Create a style preset first.'}
          </p>
        )}

        {error && <div className="banner warn" style={{ marginTop: 10 }}>Export failed: {error}</div>}
        <div className="row" style={{ marginTop: 16, justifyContent: 'flex-end', gap: 8 }}>
          <button className="ghost" onClick={onClose}>Cancel</button>
          <button className="primary" onClick={() => void doExport()} disabled={busy || createDisabled}>
            {busy ? 'Exporting…' : mode === 'update' ? 'Update song' : 'Create song'}
          </button>
        </div>
      </div>
    </div>
  );
}
