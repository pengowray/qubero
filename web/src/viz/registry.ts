// The list behind each of the four registries (`inspectorsections.ts`,
// `headingbuttons.ts`, `rowglyphs.ts`, `overviewtabs.ts`).
//
// A registry is how a diagram piece gets itself drawn by a host view without
// editing the host: the host asks the registry what to draw for the node in
// front of it, and each piece adds one entry from `register.ts`. The host
// files (`inspector.ts`, `listingdraw.ts`, `overviewpanel.ts`) are the ones
// nearly every piece would otherwise touch, and several agents editing one
// host file in parallel do not merge.
//
// Append-only on purpose: entries are drawn in the order they were added, and
// nothing takes one out, so a host never has to cope with a section that was
// there on the last render and is gone on this one. Two entries with one id
// are a mistake in `register.ts`, and fail loudly at load rather than drawing
// a piece twice.

export type Entry = { readonly id: string };

export class Registry<T extends Entry> {
  private readonly entries: T[] = [];
  private readonly what: string;

  constructor(what: string) {
    this.what = what;
  }

  add(entry: T): void {
    if (this.entries.some((e) => e.id === entry.id)) throw new Error(`${this.what}: "${entry.id}" is registered twice`);
    this.entries.push(entry);
  }

  /** Every entry, in the order added. */
  all(): readonly T[] {
    return this.entries;
  }

  get(id: string): T | undefined {
    return this.entries.find((e) => e.id === id);
  }
}
