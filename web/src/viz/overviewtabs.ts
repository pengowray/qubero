// Tabs the rail (`overviewpanel.ts`) offers beside Contents, Logical and
// B-trees.
//
// A piece that draws the whole file, such as pointer arcs over the address
// axis or a raster of the bytes at a chosen stride, is a panel in the rail,
// and it gets there by registering a tab here. The rail asks `offers` whenever
// the document changes (a template chosen, bytes arriving) and shows the tab
// only where it is offered, the way B-trees appears only for HDF5.
//
// The rail makes the panel the first time its tab is shown, not before: a
// panel that read the file to fill itself in behind a hidden tab would be a
// read nobody asked for. After that the rail keeps it, and tells it:
// - `setShown`, whenever it comes into view or goes out of it;
// - `resize`, with the width it has, whenever the rail's width changes (the
//   panel never measures the rail);
// - `reveal`, with the field the cursor is in, when the cursor moves;
// - `pump`, whenever the rail nudges its own pictures to read more.
// A click in the panel goes to the bytes through `viz/pick.ts`, the same as
// from any other piece, so the rail passes no callbacks.

import type { Doc } from "../doc.ts";
import { Registry } from "./registry.ts";

export type OverviewTabPanel = {
  readonly el: HTMLElement;
  setShown?(shown: boolean): void;
  resize?(width: number): void;
  reveal?(path: readonly number[]): void;
  pump?(): void;
};

export type OverviewTab = {
  readonly id: string;
  /** The tab's name, a word or two, like Contents. */
  readonly label: string;
  /** What the tab shows, on hover. */
  readonly title: string;
  readonly offers: (doc: Doc) => boolean;
  readonly mount: (doc: Doc, width: number) => OverviewTabPanel;
};

const TABS = new Registry<OverviewTab>("overview tab");

export function registerOverviewTab(tab: OverviewTab): void {
  TABS.add(tab);
}

export function overviewTabs(): readonly OverviewTab[] {
  return TABS.all();
}
