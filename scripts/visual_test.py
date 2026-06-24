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
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", p)) == 0


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

            pg.click("text=Library"); pg.wait_for_timeout(300)
            pg.click("text=Cyber Dreams"); pg.wait_for_timeout(500)
            pg.click("text=Builder / manage"); pg.wait_for_timeout(700)
            pg.screenshot(path=f"{OUT}/builder-manage.png", full_page=True)

            # workspace: stage nav now lives in the sidebar (.song-nav), editor full-width
            pg.click("button:has-text('Workspace')"); pg.wait_for_timeout(500)
            pg.screenshot(path=f"{OUT}/workspace.png", full_page=True)

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
            b.close()
        print(f"screenshots written to {OUT}")
        return 0
    finally:
        server.terminate()


if __name__ == "__main__":
    sys.exit(main())
