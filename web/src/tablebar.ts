// The bar above a table: what it is, what describes it, and the few things
// the reader can change about how it is read.
//
// Two halves, and they sit apart. On the near side is what the table is: its
// name, how many rows it has, the fields that describe it (a link each), and
// the sentence saying what one row is. On the far side are the controls: which
// way round the table is drawn, whether the byte addresses are shown, and the
// two buttons that take the table out of the tab, copy and export.
//
// The bar is built once and then only the sentence changes, because the one
// state it states in words is which way round the table is drawn.

import { el } from "./dom.ts";
import { TABLE } from "./strings.ts";
import { canTurn, TURN_MAX, type TablePlan } from "./tableplan.ts";

/** How many views have made a rows-or-columns choice, for naming each one's
 *  radio buttons apart. */
let arrangeGroups = 0;

export type BarOpts = {
  readonly title: string;
  readonly plan: TablePlan;
  /** Which way round the table is drawn as the bar is built. */
  readonly turned: boolean;
  /** Whether the address columns are on as the bar is built. */
  readonly addresses: boolean;
  /** The two buttons the view owns, since what they say follows the
   *  selection. */
  readonly copyButton: HTMLElement;
  readonly exporter: HTMLElement;
  readonly onTurn: (turned: boolean) => void;
  readonly onAddresses: (on: boolean) => void;
  readonly onFactPick: (path: readonly number[]) => void;
};

/** The bar, and the one line in it the view rewrites. */
export type Bar = {
  readonly el: HTMLElement;
  /** Say what one row is, for the table drawn this way round. */
  sayMeaning: (turned: boolean) => void;
};

export function tableBar(opts: BarOpts): Bar {
  const { plan } = opts;
  const rate = plan.rate !== null && plan.rate > 0 ? plan.rate : null;
  const meaning = el("span", { className: "tbl-meaning" });
  const sayMeaning = (turned: boolean): void => {
    if (rate === null) return;
    const word = plan.columnWord;
    const said = rate.toLocaleString();
    const rows = plan.rowWord;
    if (turned) meaning.textContent = word === null ? TABLE.columnMeaningPlain(rows, said) : TABLE.columnMeaning(rows, word, said);
    else meaning.textContent = word === null ? TABLE.rowMeaningPlain(rows, said) : TABLE.rowMeaning(rows, word, said);
  };

  const bar = el("header", { className: "tbl-bar" });
  bar.append(el("b", { className: "tbl-title", textContent: opts.title }));
  bar.append(el("span", { className: "tbl-count", textContent: TABLE.count(plan.count, plan.rowWord) }));
  for (const fact of plan.facts) {
    const button = el("button", {
      type: "button",
      className: "tbl-fact",
      textContent: `${fact.label} ${factValue(fact.value)}`,
    });
    button.title = TABLE.factTitle(fact.label);
    button.addEventListener("click", () => opts.onFactPick(fact.path));
    bar.append(button);
  }
  if (rate !== null) bar.append(meaning);
  sayMeaning(opts.turned);
  const box = el("input", { type: "checkbox", className: "tbl-addr-box", checked: opts.addresses });
  box.addEventListener("change", () => opts.onAddresses(box.checked));
  // The controls sit together at the far end, away from the facts.
  bar.append(el("div", { className: "tbl-controls" }, arrangeChoice(opts), el("label", { className: "tbl-check" }, box, TABLE.addresses), opts.copyButton, opts.exporter));
  return { el: bar, sayMeaning };
}

/**
 * Which way round the table is drawn, as a label and its two answers:
 * `Samples in: (o) rows ( ) columns`. The answer filled in is the state, so a
 * table that arrived turned says so without the reader working it out. A
 * table too long to turn has `columns` greyed, with the reason on hover.
 */
function arrangeChoice(opts: BarOpts): HTMLElement {
  // Several tables can be open in one page, and radio buttons that share a
  // name are one group wherever they are, so each view's name is its own.
  const name = `tbl-arrange-${++arrangeGroups}`;
  const can = canTurn(opts.plan.count);
  const choice = (turned: boolean, text: string): HTMLElement => {
    const box = el("input", { type: "radio", name, checked: opts.turned === turned, disabled: turned && !can });
    box.addEventListener("change", () => {
      if (box.checked) opts.onTurn(turned);
    });
    const label = el("label", { className: "tbl-check" }, box, text);
    if (box.disabled) {
      label.classList.add("is-off");
      label.title = TABLE.turnTooMany(opts.plan.rowWord, TURN_MAX);
    }
    return label;
  };
  const group = el("span", { className: "tbl-arrange" }, el("span", { className: "tbl-arrange-word", textContent: TABLE.arrange(opts.plan.rowWord) }), choice(false, TABLE.arrangeRows), choice(true, TABLE.arrangeColumns));
  group.setAttribute("role", "radiogroup");
  group.setAttribute("aria-label", TABLE.arrange(opts.plan.rowWord));
  group.title = TABLE.arrangeTitle(opts.plan.rowWord);
  return group;
}

/** A fact's value as the bar shows it: a number gets its thousands separators,
 *  since `44,100` is read at a glance and `44100` is counted. Anything else is
 *  shown as the core wrote it. */
export function factValue(value: string): string {
  const n = Number(value);
  return value.trim() !== "" && Number.isFinite(n) ? n.toLocaleString() : value;
}
