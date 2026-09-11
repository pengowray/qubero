# Handover: what is left open in the hex view and the B-trees panel

First written 2026-09-11 after the session that made the hex view reuse rows,
added the B-trees rail tab, and put an `@` in front of every address. Rewritten
the same day after a session that went through it and closed most of it.

Nothing here is broken on screen today. What is left are hazards, stale
assumptions and loose ends found while doing other work, written down with what
to look for. Each entry says where it is, how to see it, and what must not
break while fixing it.

## Closed

| Was | Commit |
|---|---|
| 1. `ensure()` detaches rows when the window shrinks | `dda8a41` |
| 4. Two browser assertions encode an assumption row reuse retired | `a13fb68` |
| 5. A recycled chip can carry a stale tooltip | `e239ac0` |
| 10. ELF segment virtual addresses do not carry the mark | `a0b51ac` |
| 12. `.claude/launch.json` lies to a worktree, in the port half of it | `b681f5a` |

Number 10 was the small one on the list and it was sitting on a real defect.
The line about `busybox-x86_64` reporting 0 mapped regions was not a quirk of
that file: the ELF template had grown a `section_name_base` field at header
index 13, and the Logical outline reached into the tree with literal child
indices, so every table after it was off by one. An executable whose `e_phnum`
says 4 drew no segments at all, the sections and symbols groups pointed at the
wrong tables, and a section's flags column showed its type. A second insertion,
a `name` field inside every section header, did the same one level down. Both
are read by name now. What follows in "Still open" number 2 is the general form
of that.

## The two scroll rules, which everything in the hex view is held to

Stated here because half of what follows is a threat to one of them.

1. **Never detach a row under a touch.** A drag following the finger must not
   have the element it is dragging replaced. Row reuse satisfies this
   structurally: `place()` assigns each element a flex `order` and the
   document's child order never changes. `ensure()`'s shrink path was the one
   remaining hole and is now shut while a finger is down.
2. **Never let the top row change height.** Rows are variable height (chips
   beside a row make it taller, headings above it more so). If the row at the
   top of the viewport is re-measured to a different height mid-scroll,
   everything below it jumps and the scroll position stops meaning what it
   meant a frame ago.

The ledger those rules are enforced through is `web/src/rowheights.ts`.

## Still open

### 1. Condensed mode can make the top row taller

`chipplan`'s carried-overflow puts a chip that started on an earlier row into
block 0 of the current one. In condensed mode that can make the top row taller
than it was, which is rule 2's prohibition.

**To see it:** condensed field column, scroll so a long chip run straddles the
top edge, and watch the top row's height across a notch. Condensed is a
`setRightColumn` mode (`"fields-condensed"` and friends, whatever the menu
calls them), not a separate toggle; there is no `setCondensed`.

**What to do:** the top row already has its carried chips stripped by
`planRowChips`. Work out whether the overflow path bypasses that, and if so
whether it can be made to obey it. Do not "fix" it by re-measuring the top row.

### 2. Literal child indices go stale when a template grows a field

The general form of what closed number 10. A path like `doc.templateNode([7,
14, 0, i])` is a claim about where a field sits among its siblings, and nothing
checks it: a template that gains a field in the middle silently moves every
path past it, and what comes back is a different field of the right shape.

`web/src/logicaloutline.ts` alone holds 11 `templateNode([...])` calls and 33
`nodeValue(doc, [...])` calls with literal indices in them. ISO's
`descriptor_path: vec![1, i, 3]` in `crates/wasm/src/lib.rs` is the same claim
from the Rust side. The ELF ones are gone; the rest are untested.

**What to do:** nothing wholesale. `fieldsOf` in `logicaloutline.ts` is the
shape of the fix, one structure at a time, when a format is being worked on
anyway: one `templateChildren` call keyed by name, and an error reply when a
name is not there rather than a quiet wrong answer. The Rust core already reads
this way (`named(ev, doc, path, "section_headers")` in `elf_disasm.rs`).

**To see whether one is stale:** open a file of that format and read the values
back. A wrong index usually shows as a plausible number in the wrong column,
which is why none of these were noticed.

### 3. A chip width-fit miss, not resize-driven

Row 51 of `qubero-samples/hdf4/tvattr.hdf` predicts 44px and comes out 24 on a
**fresh** 1500-wide load. This was previously written up as a resize
degradation; that attribution is wrong. It happens with no resize involved and
is a `chipfit` width-fit miss.

**The check that looks like it works and does not.** Comparing
`ledger.heightOf(row)` against the row's `offsetHeight` after a draw reports no
mismatch on any row of that file, in any column mode, at 1500 wide. That is not
the bug being absent. `heightOf` is `base + structural + the measured extra`,
and the draw measures every row it draws, so after a draw the two sides of that
comparison are the same number by construction. The same shape of trap as
number 6 below: a reading taken after the thing that sets it.

**What would mean something:** `base + structuralOf(row)` against
`offsetHeight`, on rows where `ledger.hasMeasured(row)` is still false, or the
prediction `write()` used for the row before the browser laid it out.

**What to do:** find why `chipfit` decides a chip fits when the browser then
wraps it, or the reverse. The prediction is otherwise exact, row for row,
across 870 rows of 30 draws on three sample files, so this is a narrow bug and
not a general looseness.

### 4. A box now knows what its width means and still does not say it

`Placed.weight` is the count a box's width stands for, and `Box.weight` now
carries it through to the drawn box (`5ed2006`). The line on the tooltip and
the readout that would print it is not written: `nodeLines` still gives the
node's own entry count and never says what the width is.

**What to do:** one line in `boxTitle` / `readoutLines` (`web/src/btreedraw.ts`),
after the holds/points-at pair and before the key range. The noun comes from
the tree and not from the node: records for a version 2 tree whatever the
node's kind, links for a version 1 group tree even on an index node. Keep the
wording consistent with `BTREES.widthGroup` / `widthChunk` / `widthRecords`,
which say width is proportional to what is in and below a box. Note that an
empty node is given a weight of 1 so that it still has a box to press, so the
number can read 1 where the truth is 0.

### 5. Version 2 nodes below the root cannot be opened in the Listing

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

### 6. "The number on a box is what is in the band below it" is not true of v2

A version 2 B-tree is a B-tree, not a B+ tree: a record sits between every two
children, so an internal node points at one more child than it holds records.
The root of `tree-v2.h5` prints 1 record and has 2 children.

This is what a B-tree is, not a defect, and `nodeLines` already says "points at
N children" on the readout. It is written down because the terminal band was
added to teach that rule, and the rule has an exception the drawing does not
show.

### 7. One duplicated number

The summary line says `999 link tables holding 4,000 links` while the band
below says `4,000 links`. Deliberate: the summary is the only place the counts
read without hovering. It is the panel's one repeated figure, noted so a future
reader does not take it for an oversight.

### 8. The hex/decimal toggle is deferred, and `@` was chosen for it

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

### 9. `.claude/launch.json` and a worktree: the half that is still unverified

The port half is fixed (`b681f5a`): the entries no longer set `PORT`
themselves, so the preview tool's reserved port is the one vite listens on, and
`strictPort` in `vite.config.ts` makes a taken port an error rather than a
quiet walk to the next one. That was the trap that actually bit: the tool
reported 5114, the command forced 17274, three servers already held 17274
through 17276, and vite served happily on 17277 where nobody was looking.

The original claim, that `cwd: "web"` resolves against the **main checkout** so
`preview_start` from a worktree serves main, was not re-tested this session and
is still the thing to watch. The check that catches it is unchanged and costs
nothing: **vite's startup banner prints the path and the branch it is serving
(`whereAmI` in `vite.config.ts`). Read it.**

## Tooling traps

### `draw ms` charges a draw for a layout that was coming anyway

`offsetHeight` makes the browser lay out *before answering*, so a forced layout
inside a draw is billed to the draw. `rows.heights` showed about 5.7ms a draw
and looked like half a hex draw's cost. It was not: removing the read halves
the layout *count* and changes layout ms, script ms, draws per second and
dropped frames by nothing.

Proved with a stub control; the negative result is in `heights()`'s doc
comment. **Read `browser: layout ms` and `script ms` before believing any
per-function timer.** This cost three agents. Still-open number 3 is the same
trap wearing a different hat, so it is worth reading both before measuring
anything.

### Nothing animates while the browser pane is hidden

`requestAnimationFrame` never fires and `setTimeout` is throttled hard, so a
`javascript_tool` script that scrolls and then awaits a frame times out at 45
seconds. Drive the view synchronously in one tick instead: `view.scrollToY(y);
view.render();` then measure.

## Measuring, before changing any of the above

- `web/tools/wheelcost.mjs` - draws, per-draw cost, browser style and layout
  time, per-1000px figures, and DOM mutation counts. The mutation counts are
  deterministic and are the number to trust: this machine measured 425ms and
  then 326ms on identical code forty minutes apart.
- `web/tools/touchscroll.mjs` - how closely a touch drag is followed, against
  `web/tools/touchscroll-baseline.md`. **This is the tool that would catch a
  broken rule 2**; a wheel measurement cannot see it.
- `web/tools/staleness.mjs` - drives two servers through the same script and
  diffs every visible cell, class, chip and heading. Stale content is what a
  reuse bug looks like and no timing tool sees it. Expect a few differences
  from two browsers reaching an async step at different moments; drive each one
  alone before believing it.
- `web/test/hexresize.browser.mjs` - the resize cases, 15 checks, including the
  viewport shrinking under a finger.
- `web/test/hexview.browser.mjs` - 13 checks, three of them about row reuse by
  address.

Current numbers, 2026-09-11, for comparison:

| Measurement | File | Reading |
|---|---|---|
| one wheel notch | hello.exe 1280x800 | ~60 attribute writes, 6.4ms draw |
| thirty notches | hello.exe 1280x800 | ~1,742 writes, 120ms |
| mutations per 1000px, hex | hello.exe 1280x800 | 1,867 down, 1,745 up |
| mutations per 1000px, Listing | same, for scale | 83 |
| touch following | notes.sqlite, hello.exe, bat.wav | 1.00x, 0/124 jumpy, 0.0 mean deviation, 0 stalls |
