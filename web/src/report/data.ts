// What several sections of the report need, worked out once per drawing of
// the report rather than once per section.
//
// Everything here is thrown away when the file is edited or read with another
// template: see `ReportView.reset`. A value that is still being read is not
// kept, so the next time a section asks, the core is asked again and gets
// further.

import type { Doc, OverviewState } from "../doc.ts";
import type { Directories, ExtentAudit, Ledger, Profile, ReportStep } from "./coredata.ts";
import { buildParts, partsBudget, type Budget, type PartsModel } from "./model.ts";
import { WAIT } from "./section.ts";

/** The byte-class scan's bucket count, the one the rail's map asks for. The
 *  core keeps one scan and starts it over for any other count, so asking for
 *  another would make the two views take turns throwing each other's work
 *  away. */
export const SCAN_BUCKETS = 1024;
/** How long one go at a stepped walk may take before the page gets the turn
 *  back. */
const STEP_MS = 12;

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
  private scanStepped = -1;
  private pass = 0;
  private readonly coreState: CoreReport = { profile: null, ledger: null, audit: null, dirs: null, template: undefined, failed: null };
  private coreStepped = -1;
  private coreSnapshot = 0;
  private coreMissing = false;

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
    if (this.scanState?.done === true || this.scanStepped === this.pass) return this.scanState;
    this.scanStepped = this.pass;
    const until = performance.now() + STEP_MS;
    let r = this.doc.overviewStep(SCAN_BUCKETS);
    while (r.status === "ok" && !r.node.done && performance.now() < until) r = this.doc.overviewStep(SCAN_BUCKETS);
    if (r.status === "ok") this.scanState = r.node;
    return this.scanState;
  }

  /** True while the core's walk has been started and not finished, so the
   *  view keeps giving it goes. */
  coreRunning(): boolean {
    const c = this.coreState;
    return this.coreStepped >= 0 && !this.coreMissing && c.failed === null && c.dirs?.done !== true;
  }

  /** A new pass of the view: each stepped walk may take one more go. Several
   *  sections ask for the same walk in a pass, and only the first ask steps
   *  it. */
  tick(): void {
    this.pass += 1;
  }

  /**
   * The core's four views of the file for the report, taking one more go of
   * the walk they share: the format profile, the byte ledger, the extent
   * audit and the directories, and the template's own profile beside them.
   * Each is null until the walk has answered it once, and each says `done`
   * when it is final. Null as a whole where there is no template, or where
   * the `src/pkg` in use has no such calls.
   */
  core(): CoreReport | null {
    if (this.doc.template === null || this.coreMissing) return null;
    const c = this.coreState;
    if (c.dirs?.done !== true && this.coreStepped !== this.pass) {
      this.coreStepped = this.pass;
      // The directories are found last, once the walk and the reading of the
      // gaps are over, so theirs is the `done` that says everything is.
      const until = performance.now() + STEP_MS;
      for (;;) {
        const r = this.doc.reportStep<Directories>("directories_step");
        if (r === null) {
          this.coreMissing = true;
          return null;
        }
        if (r.status === "error") {
          c.failed = r.message;
          break;
        }
        if (r.status !== "ok") break;
        c.dirs = r.node;
        if (r.node.done || performance.now() > until) break;
      }
      // What the other three have come to so far, now and then while the walk
      // runs and once more when it is over.
      const now = performance.now();
      if (c.dirs?.done === true || now - this.coreSnapshot > SNAPSHOT_MS) {
        this.coreSnapshot = now;
        const take = <T>(call: ReportStep): T | null => {
          const r = this.doc.reportStep<T>(call);
          return r !== null && r.status === "ok" ? r.node : null;
        };
        c.profile = take<Profile>("format_profile_step") ?? c.profile;
        c.ledger = take<Ledger>("byte_ledger_step") ?? c.ledger;
        c.audit = take<ExtentAudit>("extent_audit_step") ?? c.audit;
      }
    }
    if (c.template === undefined) {
      const r = this.doc.templateProfile();
      c.template = r !== null && r.status === "ok" ? r.node : null;
    }
    return c;
  }
}

/** What the core has answered so far for the report. */
export type CoreReport = {
  profile: Profile | null;
  ledger: Ledger | null;
  audit: ExtentAudit | null;
  dirs: Directories | null;
  /** The template's profile, counted over its declarations; undefined until
   *  asked. */
  template: Profile | null | undefined;
  /** The core's reason, when the walk could not be taken. */
  failed: string | null;
};

/** How often the partial answers are taken while the walk runs. */
const SNAPSHOT_MS = 600;
