# Handover: what is left open in the hex view and the B-trees panel

Written 2026-09-11, after the session that made the hex view reuse rows, added
the B-trees rail tab, and put an `@` in front of every address.

Nothing here is broken on screen today. These are hazards, stale assumptions and
loose ends found while doing other work, written down with what to look for so
the next person does not have to rediscover them. Each entry says where it is,
how to see it, and what must not break while fixing it.

## The two scroll rules, which everything in the hex view is held to

Stated here because half of what follows is a threat to one of them.

1. **Never detach a row under a touch.** A drag following the finger must not
   have the element it is dragging replaced. Row reuse satisfies this
   structurally: `place()` assigns each element a flex `order` and the
   document's child order never changes.
2. **Never let the top row change height.** Rows are variable height (chips
   beside a row make it taller, headings above it more so). If the row at the
   top of the viewport is re-measured to a different height mid-scroll,
   everything below it jumps and the scroll position stops meaning what it
   meant a frame ago.

The ledger those rules are enforced through is `web/src/rowheights.ts`.

## Hex view

### 1. `ensure()` detaches rows when the window shrinks

`HexRows.ensure()`'s shrink path calls `pop()?.remove()`. That detaches row
elements, which is rule 1's exact prohibition. It has not been seen to bite
because a shrink needs the window to get smaller, and nobody has resized
mid-drag under a finger, but the hazard is real and structural.

**To see it:** start a touch drag, then shrink the viewport during it.

**What to do:** hide surplus rows rather than removing them, the way chips are
already hidden rather than removed (`hexchips.ts` keeps elements and toggles
`hidden`, deliberately, for exactly this reason). Watch the interaction with
`place()`, which hands out elements by file-row index: a hidden element is
still in the pool and must not be handed to a visible row without being
un-hidden.

### 2. Condensed mode can make the top row taller

`chipplan`'s carried-overflow puts a chip that started on an earlier row into
block 0 of the current one. In condensed mode that can make the top row taller
than it was, which is rule 2's prohibition.

**To see it:** condensed field column, scroll so a long chip run straddles the
top edge, and watch the top row's height across a notch.

**What to do:** the top row already has its carried chips stripped by
`planRowChips`. Work out whether the overflow path bypasses that, and if so
whether it can be made to obey it. Do not "fix" it by re-measuring the top row.

### 3. A chip width-fit miss, not resize-driven

Row 51 of `qubero-samples/hdf4/tvattr.hdf` predicts 44px and comes out 24 on a
**fresh** 1500-wide load. This was previously written up as a resize
degradation ("most rows 1-2px taller, a handful a whole chip line shorter, by
-16, -20 and -21px"). That attribution is wrong: it happens with no resize
involved and is a `chipfit` width-fit miss.

**To see it:** load that file at 1500 wide and compare `ledger.heightOf(51)`
against the row's real `offsetHeight`.

**What to do:** find why `chipfit` decides a chip fits when the browser then
wraps it, or the reverse. The prediction is otherwise exact, row for row,
across 870 rows of 30 draws on three sample files, so this is a narrow bug and
not a general looseness.

### 4. Two browser assertions encode an assumption row reuse retired

`web/test/hexview.browser.mjs` fails `cellsReused` and `resizeReusesCells` on
**both** the current build and the commit before row reuse (`f8d1a82`). Both
assert cell identity by screen slot. Row reuse deliberately made identity
address-based: an element now follows the file row it holds, not the position
it occupies.

**What to do:** rewrite the assertions against address-based identity, or
delete them if `web/test/hexresize.browser.mjs` and `web/tools/staleness.mjs`
already cover what they were for. Do not "fix" the source to satisfy them.

### 5. A recycled chip can carry a stale tooltip

`rowNoteKey` keys on what a chip *says*, not which field it is. Two fields with
the same name and the same value in different structures produce the same key,
so a recycled element keeps the previous field's tooltip and click path.

This was possible before row reuse too, but a recycled element is now the only
route to it, so reuse widened the exposure without creating the bug.

**To see it:** a format with repeated identical-looking fields in different
parents; scroll one out and the other in, then hover and click the chip.

**What to do:** include the field's path, or something derived from it, in the
key. Watch the cost: the key is built per row per draw, and `wheelcost.mjs`
will show it if it becomes expensive.

## B-trees panel

### 6. A box never says what its width means

A box's tooltip gives the node's own entry count and never says what its
**width** stands for. That is precisely the fact that was wrong until this
session: a version 2 root box's width counted 1,952 records under a band saying
2,000, because `place` summed leaf records only and a version 2 B-tree holds
records at every level.

**What to do:** one line in `nodeLines` (`web/src/btreedraw.ts`) naming the
weight. `Placed.weight` is already computed and is currently read only by the
tests. Keep the wording consistent with `BTREES.widthGroup` / `widthChunk` /
`widthRecords`, which say width is proportional to what is in and below a box.

### 7. Version 2 nodes below the root cannot be opened in the Listing

The template places only the version 2 root node, so nodes below it have no
path. Clicking still goes to their bytes; double-click opens nothing, and
`BTREES.notInListing` says so.

**Why it is not a small fix:** a `BTIN`'s child pointers are
`T::bytes(E::Remaining)` in the template because a pointer's two counts are as
wide as an iteration over the tree's levels with a base-two logarithm in it
says, and the expression language has no logarithm. `hdf5_tree.rs` does that
arithmetic in Rust and reads the nodes directly. Closing the gap means giving
the template that capability, which is real format work in a declarative
language, not another walk.

### 8. "The number on a box is what is in the band below it" is not true of v2

A version 2 B-tree is a B-tree, not a B+ tree: a record sits between every two
children, so an internal node points at one more child than it holds records.
The root of `tree-v2.h5` prints 1 record and has 2 children.

This is what a B-tree is, not a defect, and `nodeLines` already says "points at
N children" on the readout. It is written down because the terminal band was
added to teach that rule, and the rule has an exception the drawing does not
show.

### 9. One duplicated number

The summary line says `999 link tables holding 4,000 links` while the band
below says `4,000 links`. Deliberate: the summary is the only place the counts
read without hovering. It is the panel's one repeated figure, noted so a future
reader does not take it for an oversight.

## Address notation

### 10. ELF segment virtual addresses do not carry the mark

`web/src/logicaloutline.ts:1002` writes a segment's virtual address from the
raw field value; `:1018` puts a section's through `formatOffset`. So segment
rows read `virtual 0x400000` beside section rows reading `virtual @0x400000`.

Pre-existing divergence that the `@` mark exposed. `busybox-x86_64` reports 0
mapped regions, so the row never drew during that work and it was left alone
rather than guessed at.

**To see it:** an ELF with mapped regions, in the Logical outline.

### 11. The hex/decimal toggle is deferred, and `@` was chosen for it

`@` was picked over `$A60` and `A60h` because both of those assert **hex** by
convention, and a base toggle would make them lie. `@` claims nothing about
base. If the toggle is built:

- It touches only what `formatOffset` / `formatAddress` produce, plus the hex
  gutter's own formatter (`hexrows.ts:548`, which does not call either) and
  `UNPACKED.bit`'s dot notation. Not `formatBytes`, not counts.
- Decimal addresses print with no thousands separator, so they cannot be
  mistaken for a count and paste straight into the goto box.
- The goto placeholder says `Go to offset (hex)` and must follow the toggle, or
  it contradicts what is on screen.

## Tooling traps

### 12. `.claude/launch.json` lies to a worktree

`cwd: "web"` resolves against the **main checkout**, so `preview_start` from a
worktree serves main, not the worktree. This has caught at least five agents;
one spent a round debugging why `/tree-v2.h5` came back as HTML.

**The check that catches it:** vite's startup banner prints the path it is
serving. Read it.

**What to do:** make the entry not lie, or add a second entry that resolves
relative to the working tree. Until then every worktree brief has to warn about
it, which is where four of the five losses came from.

### 13. `draw ms` charges a draw for a layout that was coming anyway

`offsetHeight` makes the browser lay out *before answering*, so a forced layout
inside a draw is billed to the draw. `rows.heights` showed about 5.7ms a draw
and looked like half a hex draw's cost. It was not: removing the read halves
the layout *count* and changes layout ms, script ms, draws per second and
dropped frames by nothing.

Proved with a stub control; the negative result is in `heights()`'s doc
comment. **Read `browser: layout ms` and `script ms` before believing any
per-function timer.** This cost three agents.

## Measuring, before changing any of the above

- `web/tools/wheelcost.mjs` - draws, per-draw cost, browser style and layout
  time, per-1000px figures, and DOM mutation counts. The mutation counts are
  deterministic and are the number to trust: this machine measured 425ms and
  then 326ms on identical code forty minutes apart.
- `web/tools/touchscroll.mjs` - how closely a touch drag is followed, against
  `web/tools/touchscroll-baseline.md`. Currently 1.00x ratio, 0/124 jumpy, 0.0
  mean deviation, 0 stalls on all three samples. **This is the tool that would
  catch a broken rule 2**; a wheel measurement cannot see it.
- `web/tools/staleness.mjs` - drives two servers through the same script and
  diffs every visible cell, class, chip and heading. Stale content is what a
  reuse bug looks like and no timing tool sees it. Expect a few differences
  from two browsers reaching an async step at different moments; drive each one
  alone before believing it.
- `web/test/hexresize.browser.mjs` - the resize cases, 12 checks.

Current hex numbers for comparison, `hello.exe` at 1280x800: one notch is about
60 attribute writes and a 6.4ms draw; thirty notches about 1,742 writes and
120ms; 1,979 DOM mutations per 1000px scrolled. The Listing, for scale, is 83
mutations per 1000px.
