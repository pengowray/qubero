// Generated from `Refusal::ALL` in crates/core/src/codec.rs. Do not edit by hand.
// Regenerate with: UPDATE=1 cargo test -p qubero-core --lib refusal_tags

/** Every reason the core gives for leaving a compressed run as bytes, as
 *  `TemplateNode.refused` and `Doc.openSpace` carry it. */
export const REFUSAL_TAGS = [
  "too-large-copy",
  "too-large-output",
  "too-large-lazy",
  "too-large-input",
  "too-large-zstd-frame",
  "too-large-xz-block",
  "too-many-xz-blocks",
  "too-large-zstd-window",
  "too-much-at-once",
  "failed",
  "settings",
  "unaligned",
  "progressive",
  "arithmetic",
  "lossless",
  "hierarchical",
  "12-bit",
] as const;

export type RefusalTag = (typeof REFUSAL_TAGS)[number];

/** Whether a word the core sent is a refusal this build knows. */
export function isRefusalTag(s: string): s is RefusalTag {
  return (REFUSAL_TAGS as readonly string[]).includes(s);
}
