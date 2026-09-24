// Section 1: the file's name, then one line with its size, its format linked
// to the Wikipedia article, and what identified it.
//
// The name and size are drawn at once. The rest of the line waits on the
// file(1) rules and the signature list, which take a moment the first time a
// file is opened, and is filled in when they answer.

import { wikipediaUrl } from "../signatures.ts";
import { formatIdentity } from "./identity.ts";
import { WAIT, type ReportCtx, type Section } from "./section.ts";
import { fileSizeText, RV } from "./text.ts";

export const titleSection: Section = {
  id: "title",
  render(ctx: ReportCtx): HTMLElement {
    const head = document.createElement("header");
    head.className = "rv-title";
    const h1 = document.createElement("h1");
    const name = document.createElement("code");
    name.textContent = ctx.doc.name;
    h1.append(name);
    const meta = document.createElement("p");
    meta.className = "rv-meta";
    const size = document.createElement("span");
    size.textContent = fileSizeText(ctx.doc.lengthBytes);
    meta.append(size);
    head.append(h1, meta);
    ctx.live(() => {
      const id = formatIdentity(ctx.doc, ctx.data);
      if (id === WAIT) return false;
      const parts: (Node | string)[] = [size];
      if (id.name !== null) {
        parts.push(" · ");
        if (id.wikipedia !== null) {
          const a = document.createElement("a");
          a.href = wikipediaUrl(id.wikipedia.replace(/#.*$/, "")) + (id.wikipedia.includes("#") ? `#${encodeURIComponent(id.wikipedia.split("#")[1]?.replace(/ /g, "_") ?? "")}` : "");
          a.target = "_blank";
          a.rel = "noopener";
          a.textContent = id.name;
          a.title = RV.wikipediaTitle(id.wikipedia);
          parts.push(a);
        } else parts.push(id.name);
      }
      parts.push(" · ", id.how);
      meta.replaceChildren(...parts);
      return true;
    });
    return head;
  },
};
