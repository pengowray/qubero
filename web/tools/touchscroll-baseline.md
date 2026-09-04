# Touch scroll baseline

Measured on main at `cee0958`, before the scrolling fix, with
`web/tools/touchscroll.mjs`: a 1500 px touch drag in 12 px steps, then 1.5 s of
glide, headless Chromium, `hasTouch`, `isMobile`, deviceScaleFactor 1.

Run:

    cd web
    $env:PORT='17282'; npm run dev          # background
    node tools/touchscroll.mjs --url "http://localhost:17282/?url=/samples/notes.sqlite"
    node tools/touchscroll.mjs --url "http://localhost:17282/?url=/samples/hello.exe"
    node tools/touchscroll.mjs --url "http://localhost:17282/?url=/samples/bat.wav"
    node tools/touchscroll.mjs --width 1200 --height 800   # notes.sqlite, desktop

| File | Viewport | Finger px | Content px | Ratio | Rows | Jumpy steps | Max dev | Mean dev | Stalls | Glide max jump |
|---|---|---|---|---|---|---|---|---|---|---|
| notes.sqlite | 390x844 | 1488 | 1969 | 1.32x | 61 | 124 / 124 | 130 px | 16.1 px | 63 | 24 px |
| hello.exe | 390x844 | 1488 | 1123 | 0.75x | 23 | 124 / 124 | 99 px | 16.6 px | 101 | no glide |
| bat.wav | 390x844 | 1488 | 1751 | 1.18x | 61 | 124 / 124 | 108 px | 14.3 px | 63 | 24 px |
| notes.sqlite | 1200x800 | 1488 | 772 | 0.52x | 13 | 124 / 124 | 164 px | 15.7 px | 111 | 24 px |

How to read it. A CDP touch move takes about three animation frames to make the
round trip, so the script scores one *driven step*, not one frame: content
displacement between the last sampled frame under each step, against the 12 px
the finger moved. Displacement is the median of how far every row on screen
moved, so rows entering and leaving cost nothing (no frame was dropped in any of
these runs). None of the four runs reached the end of the file. The 1200x800 run
still declares `isMobile`, since the point is a touch drag on a desktop layout.

What the numbers say:

- The view never follows the finger. Every step is off by more than 3 px, mean
  about 15, because the drag converts pixels to whole rows: the content sits
  still for a step or two, then jumps a full row. That is what the stall count
  counts.
- Distance does not match either, in both directions. On notes.sqlite the
  content overshoots the finger by a third, because a heading block passing by
  moves the rows below it further than the row count says. On hello.exe and on
  the desktop layout the drag falls badly short: 1488 px of finger buys 23 rows
  and 13 rows.
- The shortfall tracks row height. The drag divides by the measured row height,
  which changes as rows with chips or headings scroll into view, so the same
  finger travel buys wildly different distances. hello.exe rows are about three
  times the nominal 24 px, and it stalls on 101 of 124 steps.
- hello.exe never glided at all: the throw at the end of the drag produced no
  movement. Where a glide did happen it moved in whole rows, 24 px a frame.

After the fix, expect the ratio near 1.00, mean deviation near zero, stalls near
zero, and a glide that decelerates in sub-row steps.

## After (main at 2026-09-04, pixel scrolling over the row-height ledger)

Same drag (1500 px, 12 px steps). Content tracks the finger exactly in every case.

| File | Viewport | Finger | Content | Jumpy | Max dev | Mean dev | Stalls | Glide max step |
|---|---|---|---|---|---|---|---|---|
| notes.sqlite | 390x844 | 1488 | 1488 | 0/124 | 0 | 0.0 | 0 | 5 |
| hello.exe | 390x844 | 1488 | 1488 | 0/124 | 0 | 0.0 | 0 | 5 |
| bat.wav | 390x844 | 1488 | 1488 | 0/124 | 0 | 0.0 | 0 | 5 |
| notes.sqlite | 1200x800 | 1488 | 1488 | 0/124 | 0 | 0.0 | 0 | 4 |

Two things bit on the way: a redraw that detaches the element under the finger makes
Chromium fire `pointercancel` and end the drag (so rows are now refilled in place, never
rebuilt), and a chip carried in from above the view must not change the top row's height
(so it is drawn in a strip pinned over the rows instead).
