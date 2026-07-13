# Vendored soundfonts (N2 — realistic playback)

Sampled instrument notes for the Composer's "Sampled" sound set. Local
assets only — this is an offline desktop app, nothing is fetched at
runtime; the JSON files are lazy-loaded as their own Vite chunks the
first time a sampled voice is requested.

## Source & license

Extracted at build time from **gleitz/midi-js-soundfonts**
(<https://github.com/gleitz/midi-js-soundfonts>, `gh-pages` branch,
`FluidR3_GM/*-mp3.js`), **MIT license**. The underlying samples are the
**FluidR3_GM** GM soundfont by Frank Wen, also MIT-licensed.

| File | GM instrument | Range (MIDI) | Samples |
| --- | --- | --- | --- |
| `piano.json` | `acoustic_grand_piano` | 45–96 | 18 |
| `strings.json` | `string_ensemble_1` | 36–84 | 17 |
| `bass.json` | `acoustic_bass` | 21–60 | 14 |

## Format

Each file is a JSON object `{ [midiNumber: string]: base64Mp3 }` — one
velocity layer, MP3-encoded (WKWebView/Safari decodes MP3 reliably;
OGG/Vorbis it does not). To keep the assets small, only **every 3rd
semitone** in each instrument's useful Composer range is kept (the
ranges cover the fixed octave bands playback uses — melody 4–5, chords
3, bass 2 — with margin); the player picks the nearest sample and
repitches via `playbackRate` (≤ 1 semitone, inaudible at one velocity
layer).

Regeneration: download the three `-mp3.js` files from the repo above and
re-run the extraction (strip the `MIDI.Soundfont.<name> = {...}` wrapper
and its trailing comma, key by MIDI number, drop the
`data:audio/mp3;base64,` prefix, subsample every 3rd semitone in the
ranges listed).
