# Reference analysis (local-light spike)

The "perception" layer for the import-a-reference feature. Local, no Torch, no
upload — extracts tempo, key, bar-level chord candidates, and rough section
boundaries with librosa only. Output is raw and handed to a "Reference Analyst"
Claude skill (the "cognition" layer) that corrects key, labels sections by
function (from chord repetition), cleans chords, and writes Structure + Chords.

## Run (Python 3.12 — numba/llvmlite lack 3.13 wheels)

    uv venv --python 3.12 analysis/.venv
    VIRTUAL_ENV=analysis/.venv uv pip install -r analysis/requirements.txt
    analysis/.venv/bin/python analysis/analyze.py <audio_file>

## Spike findings (synthetic Am F C G / C G Am F, 120 BPM, A minor)
- Bar chords: 16/16 correct (Am F C G ×2, C G Am F ×2).
- Tempo: 117.5 (≈120, snap-to-grid).
- Key: detected C major = relative of A minor — resolved by the skill from chord context.
- Sections: raw audio boundaries noisy; derive form from chord repetition instead.
