#!/usr/bin/env python3
"""Visual smoke test: boots the frontend (browser mode + in-memory mock) and
captures screenshots of the key pages so the UI can be eyeballed/diffed.

    pip install playwright && python3 -m playwright install chromium
    python3 scripts/visual_test.py        # screenshots land in scripts/screenshots/

Runs the React/Vite app standalone (no Tauri) against the mock data, so it never
touches Claude or a real DB.

Captures the key surfaces as regression screenshots:
  library, builder, builder-manage, sheet-guitar/piano,
  workspace (sidebar SONG nav + full-width editor),
  composer (blank sketch) + composer-library (Phase 3 persistence: Save the
  sketch, then the open library panel listing saved compositions) +
  composer-fullsong (timeline + lyric sheet v2 below it: ONE section at a
  time, section chips, lines flowing left-to-right; lyric sheet v3 lays the
  timeline's chord spans FROM the lyric ChordPro placements, so a chorus
  that sings its 4-chord progression twice shows 8 blocks) +
  composer-sheet-v2 (a section chip clicked — browsing the single-section
  left-to-right sheet) +
  composer-sheet-v3-chorus (the Chorus chip: 8 chord occurrences over the
  cycled 4-chord progression, each linked to its OWN timeline span) +
  composer-fullsong-chord-selected (two-way highlight: the selected chord's
  section shown + exact chord mark in the sheet) +
  composer-export (⤴ Export dialog: resolved sections preview + update-linked
  / create-new destinations),
  song-done-cta (song marked done → the header "🎹 Open in Composer" turns
  primary; the Library row check covers the 🎹 affordance),
  lyrics-chordpro (click-to-place editor) + lyrics-autoplace (⚡ Auto-place),
  paste-modal-with-preview (📋 Paste lyrics → live parsed sections, words
  verbatim) + lyrics-after-import (the stage after Import replaced the lyrics
  and back-filled Structure),
  sheet-chordpro (exact chord-over-word alignment),
  style-flyout (right inspector overlay), renders-tab, terminal.
"""
import os, socket, subprocess, sys, time
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FRONTEND = os.path.join(ROOT, "frontend")
OUT = os.path.join(ROOT, "scripts", "screenshots")
PORT = 5173


def port_open(p):
    # Vite 6 binds the dev server to IPv6 loopback (::1) only, so probe both.
    for host in ("127.0.0.1", "::1"):
        fam = socket.AF_INET6 if ":" in host else socket.AF_INET
        with socket.socket(fam) as s:
            if s.connect_ex((host, p)) == 0:
                return True
    return False


def main():
    os.makedirs(OUT, exist_ok=True)
    server = subprocess.Popen(["npm", "run", "dev"], cwd=FRONTEND,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(80):
            if port_open(PORT):
                break
            time.sleep(0.5)
        else:
            print("dev server never came up", file=sys.stderr)
            return 1

        with sync_playwright() as p:
            b = p.chromium.launch(headless=True)
            pg = b.new_page(viewport={"width": 1400, "height": 1000})
            pg.goto(f"http://localhost:{PORT}")
            pg.wait_for_load_state("networkidle")
            pg.wait_for_timeout(400)
            pg.screenshot(path=f"{OUT}/library.png", full_page=True)

            pg.click("text=Builder"); pg.wait_for_timeout(600)
            pg.screenshot(path=f"{OUT}/builder.png", full_page=True)

            # Composer (blank sketch): the Hookpad-style visual sketchpad
            # (melody + chords + bass lanes over a shared 8-bar grid, palette
            # chips, transport). MUST still render unchanged with no ?song.
            pg.click(".nav >> text=Composer"); pg.wait_for_timeout(700)
            pg.screenshot(path=f"{OUT}/composer.png", full_page=True)

            # N1 notation view: engraved staves (VexFlow), clef selectors, chord symbols
            note_btn = pg.locator("button:has-text('Notation')")
            if note_btn.count() > 0:
                note_btn.first.click(); pg.wait_for_timeout(1500)  # lazy chunk + font load
                pg.screenshot(path=f"{OUT}/composer-notation.png", full_page=True)
                pg.locator("button:has-text('Grid')").first.click(); pg.wait_for_timeout(300)
            # select the first chord block → shades its triad tones in the melody grid
            block = pg.locator(".main [aria-label='Remove chord']")
            if block.count() > 0:
                block.first.locator("xpath=..").click(); pg.wait_for_timeout(300)
                pg.screenshot(path=f"{OUT}/composer-chord-selected.png", full_page=True)
                pg.keyboard.press("Escape"); pg.wait_for_timeout(200)

            # Phase 3 persistence: name + 💾 Save the sketch (libSQL-backed;
            # mock keeps it in memory), then 📂 Open — the library panel lists
            # the seeded sketch AND the row just saved (name / updated / ×).
            pg.fill("input[placeholder='Composition name']", "My neon sketch")
            pg.click("button:has-text('💾 Save')"); pg.wait_for_timeout(400)
            pg.click("button:has-text('📂 Open')"); pg.wait_for_timeout(400)
            pg.screenshot(path=f"{OUT}/composer-library.png", full_page=True)
            # reopen the saved sketch — loads through parseStoredComposition
            # + reidentify (the panel closes; the status flips to "saved")
            pg.click(".cmp-library button:has-text('My neon sketch')")
            pg.wait_for_timeout(500)

            pg.click("text=Library"); pg.wait_for_timeout(300)
            pg.click("text=Cyber Dreams"); pg.wait_for_timeout(500)
            pg.click("text=Arrange"); pg.wait_for_timeout(700)
            pg.screenshot(path=f"{OUT}/builder-manage.png", full_page=True)

            # workspace: stage nav now lives in the sidebar (.song-nav), editor full-width
            pg.click("button:has-text('Workspace')"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/workspace.png", full_page=True)

            # Full-song export → Composer: "Open in Composer" loads the WHOLE
            # song as one long, scrollable, multi-section timeline — labeled
            # section band, the real named chords on the chord lane, melody +
            # bass lanes spanning the song — and the ChordPro lyric SHEET
            # below the timeline (lyric sheet v2: one section at a time,
            # chord names in accent above the words they land on).
            pg.click("button:has-text('Open in Composer')"); pg.wait_for_timeout(900)
            pg.screenshot(path=f"{OUT}/composer-fullsong.png", full_page=True)
            # Lyric sheet v2: exactly ONE section shows at a time, with
            # compact clickable chips to browse; a section's lines flow
            # LEFT-TO-RIGHT as inline wrapping chunks (not stacked rows).
            assert pg.locator(".cmp-sheet-section").count() == 1, \
                "lyric sheet must show exactly one section at a time"
            chips = pg.locator(".cmp-sheet-chip")
            assert chips.count() >= 2, "lyric sheet must offer section chips to browse"
            chips.nth(1).click(); pg.wait_for_timeout(300)  # browse to the 2nd section
            assert "active" in (chips.nth(1).get_attribute("class") or ""), \
                "the clicked section chip must become the active one"
            assert pg.locator(".cmp-sheet-section").count() == 1, \
                "browsing chips must still show exactly one section"
            pg.screenshot(path=f"{OUT}/composer-sheet-v2.png", full_page=True)
            # Lyric sheet v3: chord spans are laid FROM the lyric ChordPro
            # placements, so a lyric-tagged section's timeline spans equal
            # its sheet chord occurrences 1:1 (never a name-matched twin).
            # Mock Chorus 1: the Chords stage lists C-G-Am-F ONCE, the two
            # chorus lines cycle it TWICE → 8 occurrences and 8 blocks.
            chips.filter(has_text="Chorus 1").first.click(); pg.wait_for_timeout(300)
            occ = pg.locator(".cmp-sheet-section .cp-chord.set")
            assert occ.count() == 8, \
                f"chorus lyrics cycle the 4-chord progression twice → 8 sheet occurrences, got {occ.count()}"
            cids = [occ.nth(i).get_attribute("data-cid") for i in range(occ.count())]
            assert all(cids) and len(set(cids)) == len(cids), \
                f"each sheet occurrence must link to its OWN chord span (1:1, in order), got {cids}"
            # And the timeline honestly shows the sung song: one block per
            # placement in lyric-tagged sections (Verse 12 + Pre-Chorus 4 +
            # Chorus 8) plus the instrumental Intro's progression-once 4.
            blocks = pg.locator("[aria-label='Remove chord']")
            assert blocks.count() == 28, \
                f"timeline must lay one chord span per lyric placement (4+12+4+8=28), got {blocks.count()}"
            pg.screenshot(path=f"{OUT}/composer-sheet-v3-chorus.png", full_page=True)
            chips.nth(0).click(); pg.wait_for_timeout(200)  # back to the first
            # ⤴ Export dialog (export-back-to-song): the resolved sections
            # preview (absolute chord names in the composition's key) with the
            # two destinations — Update the linked song's Chords + Structure
            # (🔒 frozen sections listed + skipped) OR Create a new song
            # (preset/title; key/bpm from the composition).
            pg.click("button:has-text('⤴ Export')"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/composer-export.png", full_page=True)
            pg.click(".modal button:has-text('Cancel')"); pg.wait_for_timeout(300)
            # two-way highlight sync: select a chord block in the timeline →
            # its section tints in the sheet + the exact chord/word is marked
            blk = pg.locator("[aria-label='Remove chord']")
            if blk.count() > 0:
                blk.first.locator("xpath=..").click(); pg.wait_for_timeout(300)
                pg.screenshot(path=f"{OUT}/composer-fullsong-chord-selected.png", full_page=True)
                pg.keyboard.press("Escape"); pg.wait_for_timeout(200)
            # save the full-song composition (row remembers its song_id →
            # "♪ song" badge), then reopen it from the library: the stored
            # blob round-trips sections + lyrics, so the section band and the
            # ChordPro sheet must still render after the reload.
            pg.click("button:has-text('💾 Save')"); pg.wait_for_timeout(400)
            pg.click("button:has-text('📂 Open')"); pg.wait_for_timeout(400)
            pg.click(".cmp-library button:has-text('Cyber Dreams')"); pg.wait_for_timeout(600)
            pg.screenshot(path=f"{OUT}/composer-fullsong-reopened.png", full_page=True)
            # back to the song to continue the rest of the captures
            pg.go_back(); pg.wait_for_timeout(500)

            # Chords stage = per-section progressions with section add/remove/reorder
            pg.click(".song-nav >> text=Chords"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/chords-sections.png", full_page=True)
            # select a chord → voicing panel with inversion cycler (‹ ›)
            cell = pg.locator(".chord-cell")
            if cell.count() > 0:
                cell.first.click(); pg.wait_for_timeout(300)
                pg.screenshot(path=f"{OUT}/chords-voicing.png", full_page=True)

            # dnd-kit pointer reorder — drag section 0's handle down past section 2
            handles = pg.locator(".drag-handle")
            if handles.count() >= 3:
                src, dst = handles.nth(0).bounding_box(), handles.nth(2).bounding_box()
                pg.mouse.move(src["x"] + 5, src["y"] + 5); pg.mouse.down()
                pg.mouse.move(src["x"] + 5, src["y"] + 20, steps=5)       # exceed activation distance
                pg.mouse.move(dst["x"] + 5, dst["y"] + 10, steps=15)
                pg.wait_for_timeout(200); pg.mouse.up(); pg.wait_for_timeout(400)
                pg.screenshot(path=f"{OUT}/chords-dnd.png", full_page=True)

            # Lyrics stage = ChordPro click-to-place editor (chords pinned over words)
            pg.click(".song-nav >> text=Lyrics"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/lyrics-chordpro.png", full_page=True)

            # 📋 Paste lyrics: paste finished lyrics → live parsed preview
            # (deterministic header split, [x] and x: styles, words VERBATIM)
            # → Import replaces the Lyrics artifact and back-fills Structure.
            paste = "\n".join([
                "[Verse 1]",
                "City lights are calling me home tonight",
                "Every street I know by heart",
                "",
                "[Chorus 1]",
                "We run until the morning finds us",
                "We run until we disappear",
                "",
                "Bridge:",
                "Hold on to the static in the air",
            ])
            pg.click("button:has-text('Paste lyrics')"); pg.wait_for_timeout(300)
            pg.fill(".modal textarea", paste)
            pg.wait_for_timeout(1000)  # debounce + dry-run parse
            pg.screenshot(path=f"{OUT}/paste-modal-with-preview.png", full_page=True)
            pg.click(".modal button:has-text('Import lyrics')"); pg.wait_for_timeout(700)
            pg.screenshot(path=f"{OUT}/lyrics-after-import.png", full_page=True)

            # Section freeze: lock the first section (🔓 → 🔒) so regeneration keeps it.
            lock = pg.locator(".card button:has-text('🔓')")
            if lock.count() > 0:
                lock.first.click(); pg.wait_for_timeout(300)
                pg.screenshot(path=f"{OUT}/lyrics-frozen.png", full_page=True)
                lock2 = pg.locator(".card button:has-text('🔒')")
                if lock2.count() > 0: lock2.first.click(); pg.wait_for_timeout(200)

            # per-section 💬 refine drawer (slides in as the right inspector flyout)
            refine = pg.locator(".card button:has-text('💬')")
            if refine.count() > 0:
                refine.first.click(); pg.wait_for_timeout(400)
                pg.screenshot(path=f"{OUT}/lyrics-refine.png", full_page=True)
                pg.click(".inspector-flyout button:has-text('✕')"); pg.wait_for_timeout(200)
            # ⚡ Auto-place spreads each section's progression across its lyrics
            pg.click("button:has-text('Auto-place')"); pg.wait_for_timeout(400)
            pg.screenshot(path=f"{OUT}/lyrics-autoplace.png", full_page=True)
            pg.click("button:has-text('save revision')"); pg.wait_for_timeout(400)

            # Sheet: inline tags render at their exact word positions (no spreading)
            pg.click("text=Sheet preview"); pg.wait_for_timeout(700)
            pg.screenshot(path=f"{OUT}/sheet-chordpro.png", full_page=True)
            pg.screenshot(path=f"{OUT}/sheet-guitar.png", full_page=True)
            pg.click("button:has-text('Piano')"); pg.wait_for_timeout(600)
            pg.screenshot(path=f"{OUT}/sheet-piano.png", full_page=True)

            # right inspector flyout (Style context overlay) + Renders tab
            pg.click("button:has-text('Workspace')"); pg.wait_for_timeout(400)
            pg.click("button:has-text('Style context')"); pg.wait_for_timeout(400)
            pg.screenshot(path=f"{OUT}/style-flyout.png", full_page=True)
            pg.click(".inspector-flyout button:has-text('✕')"); pg.wait_for_timeout(300)  # close via the flyout's own ✕
            pg.click("button:has-text('Renders')"); pg.wait_for_timeout(400)
            pg.screenshot(path=f"{OUT}/renders-tab.png", full_page=True)

            # Done-state CTA: mark the song done → the header "🎹 Open in
            # Composer" becomes the primary next step, and the Library row
            # gains a small 🎹 affordance to /composer?song=<id>.
            pg.click("button:has-text('Workspace')"); pg.wait_for_timeout(300)
            pg.click("button:has-text('Mark done')"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/song-done-cta.png", full_page=True)
            pg.click(".nav >> text=Library"); pg.wait_for_timeout(400)
            assert pg.locator(".list-item button:has-text('🎹')").count() > 0, \
                "done song's Library row must show the 🎹 Composer affordance"
            pg.click("text=Cyber Dreams"); pg.wait_for_timeout(500)
            pg.click("button:has-text('Reopen')"); pg.wait_for_timeout(400)  # restore in_progress

            # global chat terminal (docked, reachable from every page)
            pg.click("text=Chat with Claude"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/terminal.png", full_page=True)

            # Settings → Connect Claude account card: signed-in (mock starts signed
            # in), then the Log-in / paste-code state after clicking Log in
            pg.click(".nav >> text=Settings"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/connect-account.png", full_page=True)
            login_btn = pg.locator("button:has-text('Log in')")
            if login_btn.count() > 0:
                login_btn.first.click(); pg.wait_for_timeout(500)  # reveals the paste-code input + auth URL
                pg.screenshot(path=f"{OUT}/connect-account-paste-code.png", full_page=True)
            b.close()
        print(f"screenshots written to {OUT}")
        return 0
    finally:
        server.terminate()


if __name__ == "__main__":
    sys.exit(main())
