/** Whether a node's name says it is an element of a list: `[3]`, or `[3] flux`
 *  when the file names its elements (an NPY record's dtype fields, a MATLAB
 *  struct's fields, a GGUF tensor). Either way it is one of many, which is
 *  what folding a run of them and closing a long list go by.
 *
 *  A module of its own because the views that ask are tested under Node, and
 *  `doc.ts` loads the wasm module on import. */
export function isElementName(name: string): boolean {
  return /^\[\d+\](?: |$)/.test(name);
}
