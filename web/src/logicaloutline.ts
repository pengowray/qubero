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
  /** A lazy group can have children before its first page has been read. */
  readonly hasChildren: boolean;
  readonly sourcePath: readonly number[];
  readonly sourceBits: number | null;
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
  /** Work caused by opening a lazy section, while known rows stay visible. */
  readonly progressText?: string;
  /** The final column is semantic and can differ by adapter. */
  readonly sizeLabel?: string;
};

type Adapter = {
  readonly matches: (doc: Doc) => boolean;
  readonly read: (doc: Doc, expanded: ReadonlySet<string>) => TemplateReply<LogicalOutline>;
};

const LOGICAL_PAGE = 80;

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
      hasChildren: object.group,
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
      sizeLabel: "Logical size",
    },
  };
}

function nodeValue(doc: Doc, path: readonly number[]): string | null {
  const reply = doc.templateNode(path);
  return reply.status === "ok" ? reply.node.value : null;
}

function ggufItemDescription(doc: Doc, section: string, path: readonly number[], fallback: string): string {
  if (section === "/metadata") {
    const valueType = nodeValue(doc, [...path, 1])?.replace(/ \(\d+\)$/, "") ?? "value";
    if (valueType === "string") return nodeValue(doc, [...path, 2, 1]) ?? fallback;
    if (valueType === "array") {
      const element = nodeValue(doc, [...path, 2, 0])?.replace(/ \(\d+\)$/, "") ?? "value";
      const countText = nodeValue(doc, [...path, 2, 1]);
      const count = countText === null ? Number.NaN : Number(countText);
      return Number.isFinite(count) ? `${count.toLocaleString()} ${element} items` : fallback;
    }
    return nodeValue(doc, [...path, 2]) ?? fallback;
  }
  if (section === "/tensors") {
    const dimensionsReply = doc.templateNode([...path, 2]);
    const dimensions: string[] = [];
    if (dimensionsReply.status === "ok") {
      for (let i = 0; i < Math.min(8, dimensionsReply.node.child_count); i++) {
        const dimension = nodeValue(doc, [...path, 2, i]);
        if (dimension !== null) dimensions.push(Number(dimension).toLocaleString());
      }
    }
    const kind = nodeValue(doc, [...path, 3])?.replace(/ \(\d+\)$/, "") ?? "";
    return [dimensions.join(" × "), kind].filter(Boolean).join(" · ") || fallback;
  }
  return fallback;
}

function ggufOutline(doc: Doc, expanded: ReadonlySet<string>): TemplateReply<LogicalOutline> {
  const tensorsReply = doc.templateNode([2]);
  if (tensorsReply.status !== "ok") return tensorsReply;
  const metadataReply = doc.templateNode([3]);
  if (metadataReply.status !== "ok") return metadataReply;
  const tensorCount = Math.max(0, Number(tensorsReply.node.value));
  const metadataCount = Math.max(0, Number(metadataReply.node.value));
  const nodes: LogicalNode[] = [{
    id: "/",
    parentId: null,
    label: "Model",
    fullName: "/",
    depth: 0,
    group: true,
    hasChildren: true,
    sourcePath: [],
    sourceBits: 0,
    sourceText: formatOffset(0),
    value: `${metadataCount.toLocaleString()} metadata · ${tensorCount.toLocaleString()} tensors`,
    type: "GGUF",
    logicalBytes: null,
    logicalApproximate: false,
    title: "GGUF model",
  }];
  const sections = [
    {
      id: "/metadata", label: "Metadata", path: [4], count: metadataCount, type: "properties",
      sourceBits: 24 * 8, sourceText: formatOffset(24 * 8),
    },
    {
      id: "/tensors", label: "Tensor catalogue", path: [5], count: tensorCount, type: "tensor records",
      sourceBits: null, sourceText: "after metadata",
    },
    {
      id: "/data", label: "Model data", path: [6], count: tensorCount, type: "weights",
      sourceBits: null, sourceText: "aligned after catalogue",
    },
  ] as const;
  let progressText: string | undefined;
  let omitted = 0;
  for (const section of sections) {
    nodes.push({
      id: section.id,
      parentId: "/",
      label: section.label,
      fullName: section.id,
      depth: 1,
      group: true,
      hasChildren: section.count > 0,
      sourcePath: section.path,
      sourceBits: section.sourceBits,
      sourceText: section.sourceText,
      value: `${section.count.toLocaleString()} ${section.type}`,
      type: section.type,
      logicalBytes: null,
      logicalApproximate: false,
      title: `${section.label} · location follows the preceding variable-length section`,
    });
    // A later section starts after the preceding variable-length one. Work
    // through expanded sections in file order instead of starting competing
    // walks in a single render.
    if (!expanded.has(section.id) || progressText !== undefined) continue;
    const limit = Math.min(LOGICAL_PAGE, section.count);
    let added = 0;
    for (let i = 0; i < limit; i++) {
      const path = [...section.path, i];
      const reply = doc.templateNode(path);
      if (reply.status !== "ok") {
        if (reply.status === "error") progressText = reply.message;
        else {
          const estimate = doc.extentEstimate();
          progressText = estimate === null
            ? `Reading ${section.label.toLowerCase()}… ${formatBytes(reply.reachedBytes)} read so far`
            : `Reading ${section.label.toLowerCase()}… ${estimate.measured_items.toLocaleString()} of ${estimate.total_items.toLocaleString()} items · ~${formatBytes(estimate.estimated_bits / 8)}`;
        }
        break;
      }
      const item = reply.node;
      nodes.push({
        id: `${section.id}/${i}`,
        parentId: section.id,
        label: item.name || `[${i.toLocaleString()}]`,
        fullName: `${section.id}/${item.name || i}`,
        depth: 2,
        group: item.composite,
        hasChildren: false,
        sourcePath: path,
        sourceBits: item.offset_bits,
        sourceText: formatOffset(item.offset_bits),
        value: ggufItemDescription(doc, section.id, path, item.value),
        type: item.type,
        logicalBytes: item.size_bits / 8,
        logicalApproximate: false,
        title: `${item.name || `[${i}]`} · ${formatOffset(item.offset_bits)}`,
      });
      added += 1;
    }
    omitted += section.count - added;
  }
  return {
    status: "ok",
    node: {
      format: "gguf",
      title: "GGUF model",
      summary: `${metadataCount.toLocaleString()} metadata properties · ${tensorCount.toLocaleString()} tensors`,
      nodes,
      total: nodes.length + omitted,
      ...(progressText === undefined ? {} : { progressText }),
      sizeLabel: "Stored extent",
    },
  };
}

/** Adapters are intentionally independent of the view. GGUF, ZIP, SQLite,
 * RIFF and MP4 can add semantic nodes here without changing the table UI. */
const ADAPTERS: readonly Adapter[] = [
  { matches: (doc) => doc.template === "hdf5", read: (doc) => hdf5Outline(doc) },
  { matches: (doc) => doc.template === "gguf", read: ggufOutline },
];

export function hasLogicalOutline(doc: Doc): boolean {
  return ADAPTERS.some((adapter) => adapter.matches(doc));
}

export function logicalOutline(doc: Doc, expanded: ReadonlySet<string>): TemplateReply<LogicalOutline> | null {
  return ADAPTERS.find((adapter) => adapter.matches(doc))?.read(doc, expanded) ?? null;
}

export function logicalLength(node: LogicalNode): string {
  if (node.logicalBytes === null) return "—";
  return `${node.logicalApproximate ? "~" : ""}${formatBytes(node.logicalBytes)}`;
}
