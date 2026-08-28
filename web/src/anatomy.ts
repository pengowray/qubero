import { bitSizeText } from "./strings.js";

/** One stored component in the miniature form of the field-anatomy strip. */
export type AnatomyPart = {
  readonly sizeBits: number;
  readonly label: string;
  readonly gap?: boolean;
  readonly rest?: boolean;
};

/** Add the same small proportional strip to Structure or Listing. Text keeps
 * saying the total; this only makes the components immediately visible. */
export function appendAnatomy(cell: HTMLElement, parts: readonly AnatomyPart[], name: string): void {
  cell.querySelector(".length-anatomy")?.remove();
  const bar = document.createElement("span");
  bar.className = "length-anatomy";
  bar.setAttribute("role", "img");
  bar.setAttribute(
    "aria-label",
    `${name}: ${parts.map((part) => `${part.label}, ${bitSizeText(part.sizeBits)}`).join("; ")}`,
  );
  const total = Math.max(1, parts.reduce((sum, part) => sum + part.sizeBits, 0));
  for (const part of parts) {
    const mark = document.createElement("span");
    mark.className = "length-part";
    if (part.gap) mark.classList.add("is-gap");
    if (part.rest) mark.classList.add("is-rest");
    mark.style.flexGrow = String(part.sizeBits / total);
    mark.title = `${part.label}: ${bitSizeText(part.sizeBits)}`;
    bar.append(mark);
  }
  cell.append(bar);
}
