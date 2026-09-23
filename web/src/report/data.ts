// What several sections of the report need, worked out once per drawing of
// the report rather than once per section.
//
// Everything here is thrown away when the file is edited or read with another
// template: see `ReportView.reset`. A value that is still being read is not
// kept, so the next time a section asks, the core is asked again and gets
// further.

import type { Doc, KindTotals, OverviewState } from "../doc.ts";
import { buildParts, partsBudget, type Budget, type PartsModel } from "./model.ts";
import { WAIT } from "./section.ts";

/** The byte-class scan's bucket count, the one the rail's map asks for. The
 *  core keeps one scan and starts it over for any other count, so asking for
 *  another would make the two views take turns throwing each other's work
 *  away. */
export const SCAN_BUCKETS = 1024;
/** How long one go at a stepped walk may take before the page gets the turn
 *  back. */
const STEP_MS = 8;

export class ReportData {
  private readonly memos = new Map<string, unknown>();
  private readonly promises = new Map<string, Promise<unknown>>();
  private readonly settled = new Map<string, unknown>();
  /** Field descriptions of the fields the report names, in the order it first
   *  named them. Section 11 lists them. */
  readonly terms = new Map<string, string>();
  private readonly finished = new Set<string>();
  /** What is left of the time the parts may spend looking for structures
   *  placed by addresses, over every try at reading them. */
  private readonly partsTime: Budget = partsBudget();
  private scanState: OverviewState | null = null;
  private kindState: KindTotals | null = null;

  constructor(
    readonly doc: Doc,
    /** Called when something asked for asynchronously has arrived. */
    private readonly arrived: () => void,
  ) {}

  /** A value worked out once. `WAIT` is not kept, so the next ask tries
   *  again. */
  memo<T>(key: string, compute: () => T | typeof WAIT): T | typeof WAIT {
    if (this.memos.has(key)) return this.memos.get(key) as T;
    const v = compute();
    if (v !== WAIT) this.memos.set(key, v);
    return v;
  }

  /** A value that arrives later: `WAIT` until it has, and the report is drawn
   *  again when it does. A failure settles as null. */
  later<T>(key: string, start: () => Promise<T>): T | null | typeof WAIT {
    if (this.settled.has(key)) return this.settled.get(key) as T | null;
    if (!this.promises.has(key)) {
      const p = start().then(
        (v) => {
          this.settled.set(key, v);
          this.arrived();
        },
        () => {
          this.settled.set(key, null);
          this.arrived();
        },
      );
      this.promises.set(key, p);
    }
    return WAIT;
  }

  parts(): PartsModel | null | typeof WAIT {
    return this.memo("parts", () => buildParts(this.doc, this.partsTime));
  }

  /** Note that a section has drawn, for a section that has to come after it
   *  in time as well as on the page. */
  markDrawn(section: string): void {
    this.finished.add(section);
  }

  drawn(section: string): boolean {
    return this.finished.has(section);
  }

  /** Name a field the report has shown, so the terms section can say what it
   *  is. The first description of a name is the one kept. */
  term(name: string, text: string | undefined): void {
    if (text === undefined || text === "" || this.terms.has(name)) return;
    this.terms.set(name, text);
  }

  /** The byte-class scan so far, taking one more step of it. Null until the
   *  first step has answered. */
  scan(): OverviewState | null {
    if (this.scanState?.done === true) return this.scanState;
    const until = performance.now() + STEP_MS;
    let r = this.doc.overviewStep(SCAN_BUCKETS);
    while (r.status === "ok" && !r.node.done && performance.now() < until) r = this.doc.overviewStep(SCAN_BUCKETS);
    if (r.status === "ok") this.scanState = r.node;
    return this.scanState;
  }

  /** The totals of the file's bits by kind of field so far, taking one more
   *  step of the walk. */
  kinds(): KindTotals | null {
    if (this.kindState?.done === true) return this.kindState;
    if (this.doc.template === null) return null;
    const until = performance.now() + STEP_MS;
    let r = this.doc.kindTotalsStep();
    while (r.status === "ok" && !r.node.done && performance.now() < until) r = this.doc.kindTotalsStep();
    if (r.status === "ok") this.kindState = r.node;
    return this.kindState;
  }
}
