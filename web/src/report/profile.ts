// Section 10: how the format is built. The core's format profile, counted
// over the template and over this file side by side, so the report can say
// what the template allows and what this file uses: byte order and number
// widths, text, variable-length numbers, enums, flags, checksums, codecs, how
// each field is found and how its length is set. Then the choices the
// template makes by a value, where the file uses fewer cases than the
// template allows, and what a reader and a writer of the format have to know,
// one fact to a sentence. See "The format profile" in
// docs/DESIGN-report-view.md.

import type { Profile, ProfileRow } from "./coredata.ts";
import { CATEGORY, CATEGORY_ORDER, readingWriting, rowKey, rowLabel, VALUE_CATEGORIES } from "./profiletext.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { bitsText, RV } from "./text.ts";

/** Choices listed before the rest are counted. */
const CHOICES_SHOWN = 24;

type Joined = { readonly row: ProfileRow; readonly file: ProfileRow | null; readonly template: ProfileRow | null };

/** The file's rows and the template's, matched by what they are. */
export function joinProfiles(file: Profile, template: Profile | null): Joined[] {
  const out = new Map<string, { row: ProfileRow; file: ProfileRow | null; template: ProfileRow | null }>();
  for (const r of file.rows) out.set(rowKey(r), { row: r, file: r, template: null });
  for (const r of template?.rows ?? []) {
    const had = out.get(rowKey(r));
    if (had !== undefined) had.template = r;
    else out.set(rowKey(r), { row: r, file: null, template: r });
  }
  return [...out.values()];
}

export const profileSection: Section = {
  id: "profile",
  render(ctx: ReportCtx): Rendered {
    const core = ctx.data.core();
    if (core === null || core.failed !== null) return null;
    const file = core.profile;
    if (file === null || !file.done) return WAIT;
    const template = core.template ?? null;
    const joined = joinProfiles(file, template);
    if (joined.length === 0) return null;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-profile";
    const h = document.createElement("h2");
    const valueKinds = (p: "file" | "template"): number => joined.filter((j) => VALUE_CATEGORIES.has(j.row.category) && (j[p]?.fields ?? 0) > 0).length;
    const used = valueKinds("file");
    const declared = valueKinds("template");
    h.textContent = template === null || declared === 0 ? RV.profileHeadingFile(used) : RV.profileHeading(used, declared);
    sec.append(h);
    const sentences = readingWriting(file);
    if (sentences.length > 0) {
      const p = document.createElement("p");
      p.className = "rv-readwrite";
      p.textContent = sentences.join(" ");
      sec.append(p);
    }
    sec.append(profileTable(joined, template !== null));
    const choices = choiceTable(file);
    if (choices !== null) sec.append(choices);
    return sec;
  },
};

function profileTable(joined: readonly Joined[], withTemplate: boolean): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-profiletable rv-stack";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  const heads: [string, string][] = [
    [RV.profileKind, ""],
    [RV.profileFields, "rv-num"],
    [RV.profileBytes, "rv-num"],
  ];
  if (withTemplate) heads.push([RV.profileDeclared, "rv-num"]);
  for (const [text, cls] of heads) {
    const th = document.createElement("th");
    th.textContent = text;
    if (cls !== "") th.className = cls;
    hr.append(th);
  }
  thead.append(hr);
  t.append(thead);
  const body = document.createElement("tbody");
  const cell = (text: string, cls: string, label: string): HTMLTableCellElement => {
    const td = document.createElement("td");
    td.className = cls;
    if (label !== "") td.dataset.label = label;
    td.textContent = text;
    return td;
  };
  for (const cat of CATEGORY_ORDER) {
    const rows = joined.filter((j) => j.row.category === cat);
    if (rows.length === 0) continue;
    rows.sort((a, b) => (b.file?.fields ?? 0) - (a.file?.fields ?? 0) || (b.template?.fields ?? 0) - (a.template?.fields ?? 0));
    const g = document.createElement("tr");
    g.className = "rv-grouprow";
    const gh = document.createElement("td");
    gh.colSpan = heads.length;
    gh.textContent = CATEGORY[cat] ?? cat;
    g.append(gh);
    body.append(g);
    for (const j of rows) {
      const tr = document.createElement("tr");
      const inFile = j.file?.fields ?? 0;
      if (inFile === 0) tr.className = "is-unused";
      let label = rowLabel(j.row);
      // What a codec's streams came to, where the file opened any.
      if (j.file !== null && j.file.category === "codec" && j.file.unpacked_fields > 0) {
        label += `: ${RV.unpackedTo(j.file.unpacked_fields, bitsText(j.file.unpacked_bits))}`;
      }
      tr.append(
        cell(label, "rv-cell-name", ""),
        cell(inFile === 0 ? RV.notInFile : inFile.toLocaleString(), "rv-num", RV.profileFields),
        cell(j.file === null || j.file.bits === 0 ? "" : bitsText(j.file.bits).replace(/ bytes?$/, ""), "rv-num", RV.profileBytes),
      );
      if (withTemplate) tr.append(cell(j.template === null ? "" : j.template.fields.toLocaleString(), "rv-num", RV.profileDeclared));
      body.append(tr);
    }
  }
  t.append(body);
  wrap.append(t);
  const cap = document.createElement("p");
  cap.className = "rv-note";
  cap.textContent = withTemplate ? RV.profileCaption : RV.profileCaptionFile;
  wrap.append(cap);
  return wrap;
}

/** The switches this file made, where it used fewer cases than the template
 *  allows: "the template allows 34 kinds of segment body, this file uses 6". */
function choiceTable(file: Profile): HTMLElement | null {
  const used = file.choices.filter((c) => c.fields > 0 && c.taken > 0 && c.taken < c.cases);
  if (used.length === 0) return null;
  used.sort((a, b) => b.fields - a.fields);
  const box = document.createElement("div");
  const h = document.createElement("h3");
  h.textContent = RV.choicesHeading(used.length);
  box.append(h);
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-stack";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const [text, cls] of [
    [RV.choiceField, ""],
    [RV.choiceAllows, "rv-num"],
    [RV.choiceUses, "rv-num"],
    [RV.choiceFields, "rv-num"],
  ] as const) {
    const th = document.createElement("th");
    th.textContent = text;
    if (cls !== "") th.className = cls;
    hr.append(th);
  }
  thead.append(hr);
  t.append(thead);
  const body = document.createElement("tbody");
  for (const c of used.slice(0, CHOICES_SHOWN)) {
    const tr = document.createElement("tr");
    const name = document.createElement("td");
    name.className = "rv-cell-name";
    const code = document.createElement("code");
    code.textContent = c.name;
    name.append(code);
    const td = (text: string, label: string): HTMLTableCellElement => {
      const x = document.createElement("td");
      x.className = "rv-num";
      x.dataset.label = label;
      x.textContent = text;
      return x;
    };
    tr.append(name, td(c.cases.toLocaleString(), RV.choiceAllows), td(c.taken.toLocaleString(), RV.choiceUses), td(c.fields.toLocaleString(), RV.choiceFields));
    body.append(tr);
  }
  t.append(body);
  wrap.append(t);
  box.append(wrap);
  if (used.length > CHOICES_SHOWN) {
    const p = document.createElement("p");
    p.className = "rv-note";
    p.textContent = RV.moreRows(used.length - CHOICES_SHOWN, "choice");
    box.append(p);
  }
  return box;
}
