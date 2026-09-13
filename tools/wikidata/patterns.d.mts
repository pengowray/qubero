// Types for the test suite; the script itself is plain JavaScript.
export type Cleaned = { pattern: string; note: string | null } | { skip: string };
export const ENC: Record<"hex" | "ascii" | "asciiCorp" | "utf8" | "pronomSignature" | "pronom" | "guid" | "pcre2" | "posixEre", string>;
export function cleanPattern(value: string, encodings: readonly string[]): Cleaned;
export function canonicalPronom(value: string): string | null;
