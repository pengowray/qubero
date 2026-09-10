// The small icons the row buttons wear, as SVG rather than as text.
//
// These are Lucide icons, copied in as their shapes rather than pulled in as a
// package: four icons do not justify a dependency, and nothing here should be
// fetched at runtime. Lucide because every icon in it is drawn on one 24-unit
// grid at one stroke width, which is the whole reason for using a set instead
// of picking characters out of Unicode: the four sit together and none of them
// is a stray weight beside the others. It is ISC, and the notice for it is in
// `tools/notices-extra.md`, which is where a licence that belongs to no crate
// and no npm package goes.
//
// The shapes below are copied byte for byte from the icon files at
// lucide-icons/lucide commit 5d592a96aaeb4a3a09ffd8134f8f5ed878c9a0c5, under
// `icons/`. Each one says which file it came from. Redrawing a curve by eye
// gets something that looks nearly right and is wrong for good, so nothing
// here was typed from memory.
//
// The wrapper attributes are Lucide's own defaults, verbatim: `fill="none"`,
// `stroke="currentColor"`, width 2, round caps and joins. `currentColor` is
// the point of them here, since the button inherits the panel's text colour
// and the app has a light theme and a dark one.

import { svgEl } from "./dom.ts";

/** One shape of an icon: a tag and the attributes the source file gives it.
 *  A tag rather than a path everywhere, because Lucide's `copy` is drawn with
 *  a `rect` and rewriting it as a path would be redrawing it. */
type Shape = readonly [keyof SVGElementTagNameMap, Readonly<Record<string, string>>];

/** Lucide's wrapper attributes, the same on every icon in the set. */
const WRAPPER: Readonly<Record<string, string>> = {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  "stroke-width": "2",
  "stroke-linecap": "round",
  "stroke-linejoin": "round",
  // The button carries the sentence, on both `title` and `aria-label`. A
  // screen reader that also read the graphic would be reading the button
  // twice, so the graphic says nothing.
  "aria-hidden": "true",
};

function icon(shapes: readonly Shape[]): SVGSVGElement {
  const root = svgEl("svg", WRAPPER);
  for (const [tag, attrs] of shapes) root.append(svgEl(tag, attrs));
  return root;
}

/** `icons/copy.svg`: one rounded square in front of another. */
const COPY: readonly Shape[] = [
  ["rect", { width: "14", height: "14", x: "8", y: "8", rx: "2", ry: "2" }],
  ["path", { d: "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" }],
];

/** `icons/square-pen.svg`: a pen over the corner of a square. Chosen over
 *  `icons/pencil.svg`, which is the same pen with no square: copy is a square
 *  and these two sit side by side on one row, so the pair reads as a pair when
 *  both are built on the same outline. */
const EDIT: readonly Shape[] = [
  ["path", { d: "M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" }],
  ["path", { d: "M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z" }],
];

/**
 * `icons/unfold-vertical.svg` and `icons/fold-vertical.svg`: a dashed line
 * across the middle with an arrow above and below it, pointing away in one
 * and towards it in the other.
 *
 * This pair rather than a chevron pair, because the button is not a
 * disclosure. The reading it sits beside is clipped to one line, and the
 * button lets it wrap to as many lines as it needs; the other face puts it
 * back. So the axis is how tall the text is allowed to be, not whether a
 * section is open or shut, and the dashed line is what says so: it is the
 * fold the block is being let out of, with the arrows moving across it. A
 * chevron points at a boundary and promises something underneath.
 *
 * The two are mirror images on the same grid, so one button wearing first one
 * and then the other reads as one control changing state.
 */
const EXPAND: readonly Shape[] = [
  ["path", { d: "M12 22v-6" }],
  ["path", { d: "M12 8V2" }],
  ["path", { d: "M4 12H2" }],
  ["path", { d: "M10 12H8" }],
  ["path", { d: "M16 12h-2" }],
  ["path", { d: "M22 12h-2" }],
  ["path", { d: "m15 19-3 3-3-3" }],
  ["path", { d: "m15 5-3-3-3 3" }],
];

const COLLAPSE: readonly Shape[] = [
  ["path", { d: "M12 22v-6" }],
  ["path", { d: "M12 8V2" }],
  ["path", { d: "M4 12H2" }],
  ["path", { d: "M10 12H8" }],
  ["path", { d: "M16 12h-2" }],
  ["path", { d: "M22 12h-2" }],
  ["path", { d: "m15 19-3-3-3 3" }],
  ["path", { d: "m15 5-3 3-3-3" }],
];

// A fresh element each time: an icon is appended into one button, and a button
// cannot share the node another button is already showing.
export const copyIcon = (): SVGSVGElement => icon(COPY);
export const editIcon = (): SVGSVGElement => icon(EDIT);
export const expandIcon = (): SVGSVGElement => icon(EXPAND);
export const collapseIcon = (): SVGSVGElement => icon(COLLAPSE);
