// Section 2: what the format is, in the core's own plain sentences for a
// reader who has never met it (`formats/about.rs`). A format with none gets
// the sentence the file(1) rules wrote about this file, the one the toolbar
// shows, or nothing at all.

import { formatIdentity } from "./identity.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { RV } from "./text.ts";

export const aboutSection: Section = {
  id: "about",
  render(ctx: ReportCtx): Rendered {
    const id = formatIdentity(ctx.doc, ctx.data);
    if (id === WAIT) return WAIT;
    const p = document.createElement("p");
    p.className = "rv-about";
    if (id.about !== null) {
      p.textContent = id.about.text;
      return p;
    }
    if (id.rule === null || id.rule.message === "") return null;
    const q = document.createElement("q");
    q.className = "rv-rule";
    q.textContent = id.rule.message;
    p.append(`${RV.fileRuleSays} `, q);
    return p;
  },
};
