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
  lyrics-chordpro (click-to-place editor) + lyrics-autoplace (⚡ Auto-place),
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
            # select the first chord block → shades its triad tones in the melody grid
            block = pg.locator(".main [aria-label='Remove chord']")
            if block.count() > 0:
                block.first.locator("xpath=..").click(); pg.wait_for_timeout(300)
                pg.screenshot(path=f"{OUT}/composer-chord-selected.png", full_page=True)

            pg.click("text=Library"); pg.wait_for_timeout(300)
            pg.click("text=Cyber Dreams"); pg.wait_for_timeout(500)
            pg.click("text=Builder / manage"); pg.wait_for_timeout(700)
            pg.screenshot(path=f"{OUT}/builder-manage.png", full_page=True)

            # workspace: stage nav now lives in the sidebar (.song-nav), editor full-width
            pg.click("button:has-text('Workspace')"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/workspace.png", full_page=True)

            # Full-song export → Composer: "Open in Composer" loads the WHOLE
            # song as one long, scrollable, multi-section timeline — labeled
            # section band, the real named chords on the chord lane, the lyric
            # line under each chord, melody + bass lanes spanning the song.
            pg.click("button:has-text('Open in Composer')"); pg.wait_for_timeout(900)
            pg.screenshot(path=f"{OUT}/composer-fullsong.png", full_page=True)
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
