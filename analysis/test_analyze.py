#!/usr/bin/env python3
"""Unit tests for the analyzer's pure logic.

    analysis/.venv/bin/python -m unittest discover -s analysis -v

stdlib unittest on purpose — `analysis/` had no test runner, and adding a
dependency to run four tests is a worse trade than using what ships with Python.
These cover `refine_tempo`, which is pure (beat times in, tempo out) and so
needs no audio — which matters, because the render corpus it was calibrated on
is not checked in.
"""
import unittest

import numpy as np

from analyze import RESID_GATE, refine_tempo


def grid(bpm, n, jitter_frac=0.0, seed=0, skip=()):
    """n beats at `bpm`, each displaced by up to ±jitter_frac of a beat.
    `skip` drops beat indices, simulating a beat the tracker missed."""
    period = 60.0 / bpm
    rng = np.random.default_rng(seed)
    idx = [i for i in range(n) if i not in skip]
    t = np.array([i * period for i in idx], dtype=float)
    if jitter_frac:
        t = t + rng.uniform(-jitter_frac, jitter_frac, size=len(t)) * period
    return np.sort(t)


class RefineTempo(unittest.TestCase):
    def test_clean_grid_is_refined_off_the_lag_quantized_guess(self):
        """The whole point: 89.1 is what librosa's lag grid can express; the
        beat times say 90."""
        bpm, resid_ms, refined = refine_tempo(grid(90.0, 120), coarse_bpm=89.1)
        self.assertTrue(refined)
        self.assertAlmostEqual(bpm, 90.0, places=4)
        self.assertLess(resid_ms, 1.0)

    def test_slight_jitter_still_refines(self):
        """Real beat tracking is never exact — 1% of a beat must not disqualify
        it, or the refinement would never apply to real audio."""
        bpm, _resid, refined = refine_tempo(grid(120.0, 200, jitter_frac=0.01, seed=1), coarse_bpm=117.5)
        self.assertTrue(refined)
        self.assertAlmostEqual(bpm, 120.0, delta=0.5)

    def test_loose_grid_falls_back_to_coarse(self):
        """The `loki` regression: when the grid is badly tracked, the fit is
        confident nonsense, so keep the coarse value."""
        bpm, resid_ms, refined = refine_tempo(grid(108.0, 150, jitter_frac=0.20, seed=2), coarse_bpm=107.7)
        self.assertFalse(refined)
        self.assertEqual(bpm, 107.7)
        # the residual is still REPORTED — that's the uncertainty signal
        self.assertIsNotNone(resid_ms)
        self.assertGreater(resid_ms, RESID_GATE * (60.0 / 108.0) * 1000.0)

    def test_a_skipped_beat_does_not_derail_the_fit(self):
        """Per-step indexing exists for this: a missed beat scores a step of 2
        rather than shifting every later index by one."""
        bpm, _resid, refined = refine_tempo(grid(100.0, 120, skip=(57,)), coarse_bpm=99.4)
        self.assertTrue(refined)
        self.assertAlmostEqual(bpm, 100.0, places=3)

    def test_wild_disagreement_with_the_coarse_estimate_is_rejected(self):
        bpm, _resid, refined = refine_tempo(grid(90.0, 120), coarse_bpm=140.0)
        self.assertFalse(refined)
        self.assertEqual(bpm, 140.0)

    def test_too_few_beats_declines_rather_than_guessing(self):
        bpm, resid_ms, refined = refine_tempo(grid(120.0, 5), coarse_bpm=123.0)
        self.assertFalse(refined)
        self.assertEqual(bpm, 123.0)
        self.assertIsNone(resid_ms)

    def test_degenerate_input_never_raises(self):
        """The analyzer must degrade, never sink: a crash here loses the whole
        analysis, not just the tempo."""
        for beats in ([], [1.0], [0.0] * 20, [5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0]):
            bpm, _resid, refined = refine_tempo(np.asarray(beats, dtype=float), coarse_bpm=120.0)
            self.assertEqual(bpm, 120.0)
            self.assertFalse(refined)


if __name__ == "__main__":
    unittest.main()
