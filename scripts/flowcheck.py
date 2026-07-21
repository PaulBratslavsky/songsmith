#!/usr/bin/env python3
"""Tier B FLOWCHECK — walk a scratch song through the whole flow with REAL
Claude, then run deterministic coherence checks and print a report.

The mock suite (core/src/flow_tests.rs, Tier A) guards the plumbing; this
catches MODEL-QUALITY drift: a keyNote describing the wrong key, sections
wandering off the spine, lyrics leaking into the style prompt, placeholder
text surviving to the final prompt. Run it before/after skill changes.

Usage:
  python3 scripts/flowcheck.py            # scratch DB in a temp dir (safe)
  python3 scripts/flowcheck.py --keep     # keep the scratch DB for inspection

Requires: `cargo build -p mcp-shim` and a logged-in `claude` CLI
(subscription auth — ANTHROPIC_API_KEY/AUTH_TOKEN are stripped, never used).
Takes several minutes: six real generations.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# progress must stream when piped (CI, task runners) — not sit in a block buffer
sys.stdout.reconfigure(line_buffering=True)

REPO = Path(__file__).resolve().parent.parent
SHIM = REPO / "target" / "debug" / "mcp-shim"

# ---- the scripted song -------------------------------------------------------
PRESET = {
    "name": "Flowcheck Harbor",
    "genre": "dream pop",
    "mood": "aching, luminous, patient",
    "influences": "reverb-hazed guitars, slow synth swells, coastal night air",
    "key_tempo_feel": "F# minor, ~100 BPM",
    "vocal_range": "soft female alto",
    "themes": "waiting, distance, the sea at night",
}
TITLE = "Harbor Light"
INTENT = "waiting at the lighthouse every night for a love that never arrives"
KEY_ROOT, KEY_MODE, BPM = "F#", "minor", 100

NOTE_RE = r"[A-G](?:#|b)?"


class Shim:
    """One persistent mcp-shim process, JSON-RPC over stdin/stdout lines."""

    def __init__(self, db_path: str):
        env = {k: v for k, v in os.environ.items() if k not in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN")}
        env["SONGSMITH_DB"] = db_path
        self.p = subprocess.Popen(
            [str(SHIM)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, env=env, text=True, bufsize=1,
        )
        self.n = 0
        self.call("initialize", proto={"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "flowcheck", "version": "0"}})

    def call(self, method_or_tool: str, proto=None, timeout=900, **arguments):
        self.n += 1
        req = (
            {"jsonrpc": "2.0", "id": self.n, "method": method_or_tool, "params": proto}
            if proto is not None
            else {"jsonrpc": "2.0", "id": self.n, "method": "tools/call", "params": {"name": method_or_tool, "arguments": arguments}}
        )
        self.p.stdin.write(json.dumps(req) + "\n")
        self.p.stdin.flush()
        deadline = time.time() + timeout
        while time.time() < deadline:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("shim exited")
            try:
                resp = json.loads(line)
            except json.JSONDecodeError:
                continue
            if resp.get("id") != self.n:
                continue  # notifications / stale
            if "error" in resp:
                raise RuntimeError(f"{method_or_tool}: {resp['error']}")
            result = resp.get("result")
            if proto is not None:
                return result
            if isinstance(result, dict) and result.get("isError"):
                raise RuntimeError(f"{method_or_tool}: {result}")
            try:
                return json.loads(result["content"][0]["text"])
            except (KeyError, TypeError, json.JSONDecodeError, IndexError):
                return result
        raise TimeoutError(f"{method_or_tool} timed out")

    def close(self):
        try:
            self.p.stdin.close()
            self.p.terminate()
        except Exception:
            pass


# ---- checks ------------------------------------------------------------------
class Report:
    def __init__(self):
        self.rows = []

    def check(self, name: str, ok: bool, detail: str = ""):
        self.rows.append(("PASS" if ok else "FAIL", name, detail))
        print(f"  {'✓' if ok else '✗'} {name}" + (f" — {detail}" if detail and not ok else ""))

    def warn(self, name: str, detail: str = ""):
        self.rows.append(("WARN", name, detail))
        print(f"  ⚠ {name}" + (f" — {detail}" if detail else ""))

    def summary(self) -> int:
        fails = [r for r in self.rows if r[0] == "FAIL"]
        warns = [r for r in self.rows if r[0] == "WARN"]
        print("\n" + "=" * 60)
        print(f"FLOWCHECK: {len(self.rows) - len(fails) - len(warns)} passed · {len(warns)} warnings · {len(fails)} FAILED")
        for status, name, detail in self.rows:
            if status != "PASS":
                print(f"  [{status}] {name}" + (f": {detail}" if detail else ""))
        return 1 if fails else 0


def artifact(shim: Shim, stage_id: str) -> dict:
    d = shim.call("get_stage", id=stage_id)
    content = (d.get("artifact") or {}).get("content", "")
    try:
        return json.loads(content)
    except json.JSONDecodeError:
        return {"kind": "", "text": content, "data": None}


def words_only(s: str) -> list[str]:
    s = re.sub(r"\[[^\]]*\]", " ", s)  # bracket tags are direction, not words
    return re.findall(r"[a-z']+", s.lower())


def wrong_key_mentions(text: str) -> list[str]:
    out = []
    for note, mode in re.findall(rf"\b({NOTE_RE})\s+(major|minor)\b", text):
        if not (note == KEY_ROOT and mode == KEY_MODE):
            out.append(f"{note} {mode}")
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true", help="keep the scratch DB")
    args = ap.parse_args()

    if not SHIM.exists():
        print("build the shim first: cargo build -p mcp-shim", file=sys.stderr)
        return 2

    tmp = tempfile.mkdtemp(prefix="flowcheck-")
    db_path = os.path.join(tmp, "flowcheck.db")
    print(f"scratch DB: {db_path}")
    shim = Shim(db_path)
    rep = Report()
    t0 = time.time()

    try:
        preset = shim.call("create_style_preset", **PRESET)
        song = shim.call("create_song", style_preset_id=preset["id"], title=TITLE)
        shim.call("update_song_intent", id=song["id"], intent=INTENT)
        song = shim.call("get_song", id=song["id"])
        rep.check(
            "song seeds the preset's key/tempo",
            song["song"]["key_root"] == KEY_ROOT and song["song"]["key_mode"] == KEY_MODE and int(song["song"]["bpm"]) == BPM,
            f"got {song['song']['key_root']} {song['song']['key_mode']} @ {song['song']['bpm']}",
        )

        stages = sorted(song["stages"], key=lambda s: int(s["ordinal"]))
        for st in stages:
            print(f"\n→ running {st['type']} (real Claude)…")
            t = time.time()
            out = shim.call("run_stage", stage_id=st["id"])
            print(f"  done in {time.time() - t:.0f}s")
            rep.check(f"{st['type']}: first run saved directly (no draft)", bool(out.get("artifact")), json.dumps(out)[:120])
            shim.call("approve_stage", stage_id=st["id"])

        by_type = {s["type"]: s for s in stages}
        arts = {t: artifact(shim, s["id"]) for t, s in by_type.items()}
        spine = shim.call("list_sections", song_id=song["song"]["id"])
        song_after = shim.call("get_song", id=song["song"]["id"])

        # ---- global invariants ------------------------------------------------
        s = song_after["song"]
        rep.check("song facts survive the whole flow", s["key_root"] == KEY_ROOT and s["key_mode"] == KEY_MODE and int(s["bpm"]) == BPM,
                  f"got {s['key_root']} {s['key_mode']} @ {s['bpm']}")
        for t, a in arts.items():
            if "⚠" in (a.get("text") or ""):
                rep.warn(f"{t}: artifact text carries a ⚠ reconciliation warn")
            rep.check(f"{t}: no placeholder text survives", "{PASTE" not in (a.get("text") or ""))

        # ---- concept ----------------------------------------------------------
        ctext = (arts["concept"].get("text") or "") + json.dumps(arts["concept"].get("data") or {})
        kw = [w for w in re.findall(r"[a-z]{4,}", INTENT.lower())]
        hits = [w for w in kw if w in ctext.lower()]
        rep.check("concept honors the intent (≥2 keywords)", len(hits) >= 2, f"intent words found: {hits}")

        # ---- structure --------------------------------------------------------
        sdata = arts["structure"].get("data") or {}
        rep.check("structure data carries no key/bpm copies", "key" not in sdata and "bpm" not in sdata, json.dumps(sdata)[:120])
        knote = (sdata.get("keyNote") or "")
        wrong = wrong_key_mentions(knote)
        rep.check("keyNote names ONLY the song's key", not wrong, f"stale keys named: {wrong} in {knote!r}")
        rep.check("keyNote actually references the song key", KEY_ROOT in knote, knote[:80])
        rep.check("spine born with a sane section count", 3 <= len(spine) <= 12, f"{len(spine)} sections")

        # ---- chords -----------------------------------------------------------
        spine_ids = [r["id"] for r in spine]
        cdata = arts["chords"].get("data") or {}
        csecs = cdata.get("sections") or []
        rep.check("chords keyed to spine ids only", all((x.get("section_id") in spine_ids) for x in csecs), "")
        stringy = [c for x in csecs for c in x.get("chords") or [] if not isinstance(c, dict)]
        rep.check("chord entries are {name,beats} objects (normalized)", not stringy, f"bare entries: {stringy[:4]}")
        rep.check("every chords section has playable chords",
                  bool(csecs) and all(
                      x.get("chords") and all(
                          isinstance(c, dict) and re.match(rf"^{NOTE_RE}", c.get("name", "")) for c in x["chords"]
                      ) for x in csecs
                  ), "")
        covered = {x.get("section_id") for x in csecs}
        missing = [r["label"] for r in spine if r["id"] not in covered]
        if missing:
            rep.warn("spine sections without chords", ", ".join(missing))

        # ---- lyric spec -------------------------------------------------------
        spec = arts["lyric_spec"].get("data") or {}
        rep.check("lyric spec is emotionally specified (hook/premise/pov/arc)",
                  all((spec.get(k) or "").strip() for k in ("hook", "premise", "pov", "arc")), json.dumps(spec)[:120])

        # ---- lyrics -----------------------------------------------------------
        ldata = arts["lyrics"].get("data") or {}
        lsecs = ldata.get("sections") or []
        rep.check("lyrics keyed to spine ids only", all((x.get("section_id") in spine_ids) for x in lsecs), "")
        sung = [x for x in lsecs if any(w for l in x.get("lines", []) for w in words_only(l))]
        rep.check("the song actually sings (≥2 sections with words)", len(sung) >= 2, f"{len(sung)} sung sections")
        paren_lines = [l for x in lsecs for l in x.get("lines", []) if re.search(r"\([^)]*\)", l)]
        if len(paren_lines) > 6:
            rep.warn("heavy parenthetical use in lyrics", f"{len(paren_lines)} lines with (…)")

        # ---- generation prompt ------------------------------------------------
        pdata = arts["prompt"].get("data") or {}
        vocal, style, tagged = pdata.get("vocalPrompt") or "", pdata.get("stylePrompt") or "", pdata.get("taggedLyrics") or ""
        rep.check("vocal prompt produced", bool(vocal.strip()), "")
        rep.check("no brackets in vocal/style prompts", "[" not in vocal and "[" not in style, (vocal + " | " + style)[:100])
        rep.check("style prompt carries key + BPM", KEY_ROOT in style and str(BPM) in style, style[:120])
        tagged_words = " ".join(words_only(tagged))
        missing_lines = []
        for x in lsecs:
            for l in x.get("lines", []):
                # LEADING/whole-line parentheticals are delivery direction, not
                # sung words — the prompt stage legitimately re-tags them as
                # [brackets] (parens are SUNG in generators). Compare the words
                # that remain after stripping the leading direction.
                stripped = re.sub(r"^\s*\([^)]*\)\s*", "", l)
                if not stripped.strip():
                    continue
                w = " ".join(words_only(stripped))
                if w and w not in tagged_words:
                    missing_lines.append(l)
        rep.check("tagged lyrics carry every sung line verbatim", not missing_lines, f"{len(missing_lines)} missing, e.g. {missing_lines[:2]}")

    finally:
        shim.close()
        if not args.keep:
            import shutil
            shutil.rmtree(tmp, ignore_errors=True)
        else:
            print(f"\nkept scratch DB: {db_path}")

    print(f"\ntotal: {time.time() - t0:.0f}s")
    return rep.summary()


if __name__ == "__main__":
    sys.exit(main())
