// What a stretch of the listing still being read says. Kept apart from
// listingdraw.ts, which cannot be loaded without the editor's wasm, so that
// node tests can check the words a placeholder gets.

import { percentText } from "./format.ts";
import { REPORT } from "./strings.ts";
import type { Item } from "./flatten.ts";

/** What a stretch of the listing still being read says: how far the core has
 *  got unpacking the large stream its fields wait on, while they wait on one;
 *  that the fields are being read, while the core works on them with no
 *  progress to give; and otherwise that bytes are on their way. The stream is
 *  the one this stretch's own fields were asked for, which is not always the
 *  one unpacked most recently: Open unpacked may be carrying on another. */
export function pendingText(item: Extract<Item, { kind: "pending" }>): string {
  const u = item.unpacking;
  if (u !== null) return REPORT.unpacking(percentText(u.read_bits, u.run_bits));
  return item.working ? REPORT.working : REPORT.reading;
}
