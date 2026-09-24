// What every section of the report is given, and what it gives back.
//
// A section is one module under `report/`, one per section of the generic
// report in docs/DESIGN-report-view.md. It is asked to draw itself whenever
// the report is drawn, and answers with its element, with nothing (the section
// is left out, which is the rule for a section with nothing to say), or with
// `WAIT`: what it needs is still being read, and asking again once the file
// has changed will get further. The view keeps each section in a slot of its
// own, so a section that fills in late does not move the ones above it.

import type { Doc } from "../doc.ts";
import type { ReportData } from "./data.ts";

/** The answer of a section still waiting on the file. */
export const WAIT = Symbol("wait");

export type Rendered = HTMLElement | null | typeof WAIT;

/** A place in the file a reference points at: a field when there is one, and
 *  its bits either way. */
export type Target = {
  readonly path?: readonly number[];
  readonly startBit: number;
  readonly endBit: number;
};

/** What the report asks of the page around it. Every one of these is what
 *  another view already does, so the report never moves the cursor its own
 *  way. */
export type ReportHost = {
  /** A single click on a byte reference or a part: move the cursor there, so
   *  the panel at the cursor says what it is. The report stays on screen. */
  pick(t: Target): void;
  /** A double click: go to the hex view with the cursor there. */
  go(t: Target): void;
  /** Open a list as a table in a tab of its own. */
  openTable(path: readonly number[]): void;
  /** Go to the listing with this field in view. */
  showInListing(path: readonly number[]): void;
  /** Open a compressed run as a document of its own. */
  openUnpacked(path: readonly number[]): void;
};

export type ReportCtx = {
  readonly doc: Doc;
  readonly host: ReportHost;
  readonly data: ReportData;
  /** Keep this figure up to date as the file is read: called after every
   *  change to the document until it returns true. */
  live(update: () => boolean): void;
};

export type Section = {
  /** Which section this is, for the slot it is drawn into. */
  readonly id: string;
  render(ctx: ReportCtx): Rendered;
};
