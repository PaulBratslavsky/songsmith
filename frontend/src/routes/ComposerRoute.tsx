import { Composer } from "../components/compose/Composer";

export function ComposerRoute() {
  return (
    <div>
      <div className="topbar">
        <div>
          <h1>Composer</h1>
          <span className="muted">
            Hookpad-style 8-bar sketchpad — lay down chords, melody, and bass in scale
            degrees over a shared grid and loop it through the synth. Change the key and
            everything transposes.
          </span>
        </div>
      </div>
      <Composer />
    </div>
  );
}
