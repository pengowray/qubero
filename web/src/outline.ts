// The parts of the file, as every surface names them.
//
// The listing works out what the parts of a file are (`sectionBreaks` in
// `flatten.ts`) and draws them as headings. The rail lists the same headings,
// the hex view draws them as heading lines inside its scroll, and every file
// map's segments are the top-level ones. One list, so the four never disagree
// about what the parts are called or where they start.

/** One heading of the listing: a top-level part of the file (`level` 0) or a
 *  named part inside one (`level` 1). */
export type OutlineHeading = {
  /** The listing item's key, so a click can be answered by the listing. */
  readonly key: string;
  /** Which top-level part this is, or is in. Its colour comes from this. */
  readonly section: number;
  readonly level: 0 | 1;
  readonly path: readonly number[];
  readonly name: string;
  readonly offsetBits: number;
  readonly sizeBits: number;
  readonly color: string;
};

/** The stretch of the file a view is showing, as bits. */
export type Viewport = { readonly startBit: number; readonly endBit: number };
