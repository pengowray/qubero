import { formatBytes, formatOffset } from "./doc.js";
import type { ContentObject, Doc, TemplateReply } from "./doc.js";

/** One format-independent entry in a file's semantic outline. `sourcePath`
 * connects it back to the storage template without making that template's
 * wrappers the shape of this tree. */
export type LogicalNode = {
  readonly id: string;
  readonly parentId: string | null;
  readonly label: string;
  readonly fullName: string;
  readonly depth: number;
  readonly group: boolean;
  readonly sourcePath: readonly number[];
  readonly sourceBits: number;
  readonly sourceText: string;
  readonly value: string;
  readonly type: string;
  readonly logicalBytes: number | null;
  readonly logicalApproximate: boolean;
  readonly title: string;
};

export type LogicalOutline = {
  readonly format: string;
  readonly title: string;
  readonly summary: string;
  readonly nodes: readonly LogicalNode[];
  readonly total: number;
};

type Adapter = {
  readonly matches: (doc: Doc) => boolean;
  readonly read: (doc: Doc) => TemplateReply<LogicalOutline>;
};

function shapeCount(shape: readonly number[]): number | null {
  if (shape.length === 0) return null;
  let count = 1;
  for (const dim of shape) {
    count *= dim;
    if (!Number.isSafeInteger(count)) return null;
  }
  return count;
}

function elementBytes(element: string): number | null {
  const numeric = /^(?:u|i|f|bf)(8|16|32|64|128)(?:\b|$)/i.exec(element);
  if (numeric !== null) return Number(numeric[1]) / 8;
  const fixed = /^(\d+)-byte\b/i.exec(element);
  if (fixed !== null) return Number(fixed[1]);
  if (/^byte\b/i.test(element)) return 1;
  return null;
}

function ownLogicalExtent(object: ContentObject): { readonly bytes: number | null; readonly approximate: boolean } {
  const count = shapeCount(object.shape);
  const each = elementBytes(object.element);
  if (count !== null && each !== null) {
    const bytes = count * each;
    return { bytes: Number.isSafeInteger(bytes) ? bytes : null, approximate: false };
  }
  // With no usable shape/type pair, the physical payload is the closest
  // available value, but it must not masquerade as an exact decoded size.
  return { bytes: object.bytes > 0 ? object.bytes : null, approximate: object.bytes > 0 };
}

function baseName(name: string): string {
  if (name === "/") return "File contents";
  return name.slice(name.lastIndexOf("/") + 1) || name;
}

function parentName(name: string): string | null {
  if (name === "/") return null;
  const at = name.lastIndexOf("/");
  return at <= 0 ? "/" : name.slice(0, at);
}

function hdf5Outline(doc: Doc): TemplateReply<LogicalOutline> {
  const reply = doc.contents();
  if (reply.status !== "ok") return reply;
  const contents = reply.node;
  const own = new Map(contents.objects.map((object) => [object.name, ownLogicalExtent(object)]));
  const nodes = contents.objects.map((object): LogicalNode => {
    const ownExtent = own.get(object.name) ?? { bytes: null, approximate: false };
    let logicalBytes = ownExtent.bytes;
    let approximate = ownExtent.approximate;
    if (object.group) {
      let total = 0;
      let found = false;
      let unknown = false;
      const prefix = object.name === "/" ? "/" : `${object.name}/`;
      for (const child of contents.objects) {
        if (child.name === object.name || !child.name.startsWith(prefix) || child.group) continue;
        const extent = own.get(child.name) ?? { bytes: null, approximate: false };
        if (extent.bytes === null) unknown = true;
        else {
          total += extent.bytes;
          found = true;
          approximate ||= extent.approximate;
        }
      }
      logicalBytes = found ? total : null;
      approximate = unknown;
    }
    const shape = object.shape.map((d) => d.toLocaleString()).join(" × ");
    const chunks =
      object.storage === "chunked"
        ? `chunks ${object.chunk_dims.map((d) => d.toLocaleString()).join(" × ")}`
        : object.storage === "contiguous"
          ? "contiguous"
          : object.storage === "compact"
            ? "compact"
            : "";
    const value = [shape, chunks, object.filters.join(" then ")].filter(Boolean).join(" · ");
    const type = object.encoding || (object.group ? "group" : object.element || "dataset");
    const depth = object.name === "/" ? 0 : object.name.split("/").filter(Boolean).length;
    return {
      id: object.name,
      parentId: parentName(object.name),
      label: baseName(object.name),
      fullName: object.name,
      depth,
      group: object.group,
      sourcePath: object.path,
      sourceBits: object.address * 8,
      sourceText: object.group || object.storage === "chunked" ? "multiple" : formatOffset(object.address * 8),
      value,
      type,
      logicalBytes,
      logicalApproximate: approximate,
      title: `${object.name} · object header at ${formatOffset(object.address * 8)}`,
    };
  });
  const said = contents.encoding === "anndata" ? "AnnData" : contents.anndata ? "Looks like AnnData" : "HDF5";
  const dimensions =
    contents.rows > 0 && contents.columns > 0
      ? `${contents.rows.toLocaleString()} observations × ${contents.columns.toLocaleString()} variables`
      : "";
  return {
    status: "ok",
    node: {
      format: "hdf5",
      title: contents.anndata ? "AnnData contents" : "HDF5 contents",
      summary: [said, dimensions, `${contents.total.toLocaleString()} objects`].filter(Boolean).join(" · "),
      nodes,
      total: contents.total,
    },
  };
}

/** Adapters are intentionally independent of the view. GGUF, ZIP, SQLite,
 * RIFF and MP4 can add semantic nodes here without changing the table UI. */
const ADAPTERS: readonly Adapter[] = [{ matches: (doc) => doc.template === "hdf5", read: hdf5Outline }];

export function hasLogicalOutline(doc: Doc): boolean {
  return ADAPTERS.some((adapter) => adapter.matches(doc));
}

export function logicalOutline(doc: Doc): TemplateReply<LogicalOutline> | null {
  return ADAPTERS.find((adapter) => adapter.matches(doc))?.read(doc) ?? null;
}

export function logicalLength(node: LogicalNode): string {
  if (node.logicalBytes === null) return "—";
  return `${node.logicalApproximate ? "~" : ""}${formatBytes(node.logicalBytes)}`;
}
