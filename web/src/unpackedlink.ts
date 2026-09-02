// Turning the codec's map into the mark one tab puts on another.
//
// Two questions, two answers, and they are not the same shape. From a byte of
// an unpacked stream the question is which bits of the compressed run produced
// it: the answer is a step, counted in bits of the file. From a bit of the file
// the question is which unpacked bytes it came to: the answer is a range,
// counted in bytes of the space. Both end up as a bit range, because that is
// what a hex view marks, and the conversion is the only thing here.
//
// It is its own module so it can be read and tested without a document: the map
// behind it is package A's, and until its decoders keep a trace every answer is
// null. What is settled now is what happens to an answer once there is one.

/** The bits of the compressed run a step read, and the bytes it produced. */
export type Step = {
  readonly in_start: number;
  readonly in_end: number;
  readonly out_start: number;
  readonly out_end: number;
  readonly kind: string;
  readonly field?: string;
  readonly value?: number;
  readonly len?: number;
  readonly dist?: number;
};

/** A stretch of one document, in bits of it. */
export type Marked = { readonly startBit: number; readonly endBit: number };

/**
 * Where to mark the compressed run, given what produced the byte under the
 * cursor of an unpacked tab. Already in bits, because a compressed field need
 * not start on a byte: a deflate literal is a few bits in the middle of one.
 */
export function markFromStep(step: Step | null): Marked | null {
  if (step === null) return null;
  // A step that read no bits marks nothing. It is not an error: a match copies
  // from what came before and a decoder may charge its bits to the token.
  if (step.in_end <= step.in_start) return null;
  return { startBit: step.in_start, endBit: step.in_end };
}

/**
 * Where to mark the unpacked stream, given the step that read the bits under
 * the cursor of the compressed tab. Counted in bytes over there, because
 * unpacked output is bytes: nothing produces half of one.
 *
 * A step that produced nothing marks nothing. That is not a failure: a block
 * header is read and produces no output, and outlining zero bytes of the
 * unpacked stream would say the reader is standing somewhere they are not.
 */
export function markFromRange(step: Step | null): Marked | null {
  if (step === null) return null;
  if (step.out_end <= step.out_start) return null;
  return { startBit: step.out_start * 8, endBit: step.out_end * 8 };
}
