// The one hover box the report shows exact values in: the bytes behind a
// reference, a part of the map, a band of a ribbon. It is the same box every
// diagram piece uses, `viz/tip.ts`, where it moved when the pieces needed it
// too; the report's figures import it from here as they always have.

export { hideTip, tipAt, tipBy } from "../viz/tip.ts";
