import { formatBytes, formatOffset } from "./doc.js";
import type { ContentObject, Doc, TemplateReply } from "./doc.js";
import { countText } from "./strings.js";

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
  readonly more?: readonly LogicalMore[];
};

export type LogicalMore = {
  readonly sectionId: string;
  readonly afterId: string;
  readonly count: number;
  readonly label: string;
};

type Adapter = {
  readonly matches: (doc: Doc) => boolean;
  readonly read: (
    doc: Doc,
    expanded: ReadonlySet<string>,
    shown: ReadonlyMap<string, number>,
  ) => TemplateReply<LogicalOutline>;
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

function ggufOutline(
  doc: Doc,
  expanded: ReadonlySet<string>,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> {
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
  const more: LogicalMore[] = [];
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
    const limit = Math.min(shown.get(section.id) ?? LOGICAL_PAGE, section.count);
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
    if (added < section.count) {
      more.push({
        sectionId: section.id,
        afterId: added === 0 ? section.id : `${section.id}/${added - 1}`,
        count: section.count - added,
        label: section.type,
      });
    }
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
      ...(more.length === 0 ? {} : { more }),
      sizeLabel: "Stored extent",
    },
  };
}


// ----- Zarr stores written into a ZIP (a "ZipStore") -----

/** The file names a Zarr store uses for its own bookkeeping: three from v2,
 * one from v3, which folds group, array and attributes into a single file. */
const ZARR_METADATA = new Set([".zarray", ".zgroup", ".zattrs", "zarr.json"]);
/** Entries whose names are read to work out what the store holds. A store of
 * a million chunks is counted up to here and said to be at least that big,
 * which beats making someone wait for a number they already believe. */
const ZARR_SCAN = 20_000;
/** Metadata files parsed, and the most that is read from any one of them.
 * Both are far above what a real store needs and stop a crafted archive from
 * turning the outline into a download. */
const ZARR_METADATA_FILES = 400;
const ZARR_METADATA_BYTES = 256 * 1024;

type ZipEntry = {
  readonly path: readonly number[];
  /** The archive entry's name, with forward slashes and no leading one. */
  readonly name: string;
  readonly leaf: string;
  /** The folder the entry sits in: `image.zarr/0` for `image.zarr/0/.zarray`. */
  readonly prefix: string;
  readonly stored: boolean;
  readonly compression: string;
  readonly compressedBytes: number;
  readonly unpackedBytes: number;
  readonly offsetBits: number;
};

type ZarrNode = {
  prefix: string;
  kind: "group" | "array";
  /** What `.zarray`, `.zgroup` or `zarr.json` said, where it could be read. */
  meta: Record<string, unknown> | null;
  attributes: Record<string, unknown> | null;
  metadataPath: readonly number[];
  metadataOffsetBits: number;
  unreadMetadata: boolean;
  chunks: ZipEntry[];
};

function jsonObject(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function numberList(value: unknown): number[] {
  return Array.isArray(value) ? value.filter((n): n is number => typeof n === "number") : [];
}

/** The folder part of an entry's name, which is the Zarr node it belongs to. */
function folderOf(name: string): string {
  const at = name.lastIndexOf("/");
  return at < 0 ? "" : name.slice(0, at);
}

/** How wide one element is, from either generation's way of saying so.
 * v2 writes a NumPy descriptor (`<u2`, `|b1`); v3 writes a name (`uint16`). */
function zarrElement(meta: Record<string, unknown> | null): { readonly name: string; readonly bytes: number | null } {
  const v2 = meta?.["dtype"];
  if (typeof v2 === "string") {
    const parsed = /^([<>|=]?)([biufcSU])(\d+)$/.exec(v2);
    if (parsed === null) return { name: v2, bytes: null };
    const kind = parsed[2] ?? "";
    const width = Number(parsed[3]);
    const letters: Record<string, string> = { b: "bool", i: "int", u: "uint", f: "float", c: "complex" };
    const bytes = kind === "U" ? width * 4 : width;
    const name = kind in letters ? `${letters[kind]}${kind === "b" ? "" : width * 8}` : v2;
    return { name, bytes };
  }
  const v3 = meta?.["data_type"];
  if (typeof v3 === "string") {
    const width = /(\d+)$/.exec(v3);
    const bits = width === null ? null : Number(width[1]);
    return { name: v3, bytes: v3 === "bool" ? 1 : bits === null ? null : bits / 8 };
  }
  const named = jsonObject(v3)?.["name"];
  return typeof named === "string" ? { name: named, bytes: null } : { name: "", bytes: null };
}

/** The chunk an array is cut into, from either generation. */
function zarrChunkShape(meta: Record<string, unknown> | null): number[] {
  const v2 = numberList(meta?.["chunks"]);
  if (v2.length > 0) return v2;
  const grid = jsonObject(meta?.["chunk_grid"]);
  return numberList(jsonObject(grid?.["configuration"])?.["chunk_shape"]);
}

/** What the chunks were written through, in the order they were applied. */
function zarrCodecs(meta: Record<string, unknown> | null): string[] {
  const names: string[] = [];
  const compressor = jsonObject(meta?.["compressor"]);
  if (compressor !== null && typeof compressor["id"] === "string") names.push(compressor["id"]);
  for (const filter of Array.isArray(meta?.["filters"]) ? (meta["filters"] as unknown[]) : []) {
    const id = jsonObject(filter)?.["id"];
    if (typeof id === "string") names.push(id);
  }
  for (const codec of Array.isArray(meta?.["codecs"]) ? (meta["codecs"] as unknown[]) : []) {
    const name = jsonObject(codec)?.["name"];
    if (typeof name === "string" && name !== "bytes") names.push(name);
  }
  return names;
}

/** The OME-NGFF block, which v0.5 nests under `ome` and earlier versions
 * write straight into the attributes. */
function omeBlock(attributes: Record<string, unknown> | null): Record<string, unknown> | null {
  if (attributes === null) return null;
  const nested = jsonObject(attributes["ome"]);
  if (nested !== null) return nested;
  return Array.isArray(attributes["multiscales"]) ? attributes : null;
}

/** The axes an OME-Zarr image is laid out along: `t, c, z, y, x`. */
function omeAxes(ome: Record<string, unknown> | null): string[] {
  const multiscales = Array.isArray(ome?.["multiscales"]) ? (ome["multiscales"] as unknown[]) : [];
  const axes = jsonObject(multiscales[0])?.["axes"];
  if (!Array.isArray(axes)) return [];
  return axes
    .map((axis) => (typeof axis === "string" ? axis : jsonObject(axis)?.["name"]))
    .filter((name): name is string => typeof name === "string");
}

/** How many resolution levels the image was written at. */
function omeLevels(ome: Record<string, unknown> | null): number {
  const multiscales = Array.isArray(ome?.["multiscales"]) ? (ome["multiscales"] as unknown[]) : [];
  const datasets = jsonObject(multiscales[0])?.["datasets"];
  return Array.isArray(datasets) ? datasets.length : 0;
}

/** Every local file entry in the archive, with what its header says about it.
 * Names stop at `ZARR_SCAN`, which is what makes a store of a million chunks
 * open at all. */
function zipEntries(doc: Doc): TemplateReply<{ readonly entries: ZipEntry[]; readonly total: number; readonly partial: boolean }> {
  const recordsReply = doc.templateNode([0]);
  if (recordsReply.status !== "ok") return recordsReply;
  const entries: ZipEntry[] = [];
  let total = 0;
  let partial = false;
  for (let i = 0; i < recordsReply.node.child_count; i++) {
    const signature = doc.templateNode([0, i, 0]);
    if (signature.status !== "ok") return signature;
    if (!signature.node.value.startsWith("local file")) continue;
    total += 1;
    if (entries.length >= ZARR_SCAN) {
      partial = true;
      continue;
    }
    const body = [0, i, 1];
    const nameNode = doc.templateNode([...body, 10]);
    if (nameNode.status !== "ok") return nameNode;
    const name = nameNode.node.value.replace(/\\/g, "/").replace(/^\/+/, "");
    const compression = nodeValue(doc, [...body, 2])?.replace(/ \(\d+\)$/, "") ?? "";
    entries.push({
      path: [0, i],
      name,
      leaf: name.slice(name.lastIndexOf("/") + 1),
      prefix: folderOf(name),
      stored: compression === "stored",
      compression: compression || "file",
      compressedBytes: Number(nodeValue(doc, [...body, 6]) ?? 0),
      unpackedBytes: Number(nodeValue(doc, [...body, 7]) ?? 0),
      offsetBits: signature.node.offset_bits,
    });
  }
  return { status: "ok", node: { entries, total, partial } };
}


/** What a chunk key says about which chunk it is, as the grid coordinate the
 * store wrote. v2 names a chunk `0.1.2` under the array's folder; v3 nests it
 * as `c/0/1/2`. Both come back as `0.1.2`, so a reader sees the coordinate
 * rather than the last path segment on its own. */
function chunkCoordinate(arrayPrefix: string, name: string): string {
  const inside = arrayPrefix === "" ? name : name.slice(arrayPrefix.length + 1);
  const parts = inside.split("/").filter(Boolean);
  if (parts[0] === "c") parts.shift();
  const joined = parts.join(".");
  // A zero-dimensional array's one chunk is written as `c` alone.
  return joined === "" ? inside : joined;
}

/** Read one stored metadata entry and parse it. Returns null when the entry
 * is compressed, too large, or not JSON; the row for it then says the store
 * was not read rather than showing a shape nobody wrote. */
function readZarrMetadata(doc: Doc, entry: ZipEntry): { readonly meta: Record<string, unknown> | null; readonly pending: boolean } {
  if (!entry.stored || entry.compressedBytes === 0 || entry.compressedBytes > ZARR_METADATA_BYTES) {
    return { meta: null, pending: false };
  }
  const dataNode = doc.templateNode([...entry.path, 1, 12]);
  if (dataNode.status !== "ok") return { meta: null, pending: true };
  const { bytes, complete } = doc.read(dataNode.node.offset_bits / 8, entry.compressedBytes);
  if (!complete) return { meta: null, pending: true };
  try {
    return { meta: jsonObject(JSON.parse(new TextDecoder().decode(bytes))), pending: false };
  } catch {
    return { meta: null, pending: false };
  }
}

/** Turn the archive's entries into the tree the store is written as: a node
 * per group and array, and every other entry counted against the array whose
 * folder it sits under. */
function zarrNodes(
  doc: Doc,
  entries: readonly ZipEntry[],
): TemplateReply<{ readonly nodes: Map<string, ZarrNode>; readonly unowned: number }> {
  const nodes = new Map<string, ZarrNode>();
  const at = (prefix: string, entry: ZipEntry): ZarrNode => {
    const found = nodes.get(prefix);
    if (found !== undefined) return found;
    const made: ZarrNode = {
      prefix, kind: "group", meta: null, attributes: null,
      metadataPath: entry.path, metadataOffsetBits: entry.offsetBits, unreadMetadata: false, chunks: [],
    };
    nodes.set(prefix, made);
    return made;
  };
  let parsed = 0;
  for (const entry of entries) {
    if (!ZARR_METADATA.has(entry.leaf)) continue;
    const node = at(entry.prefix, entry);
    // Which node this is comes from the file name in v2, so it holds even when
    // the file itself cannot be read. v3 writes one file for both and has to
    // be read to say which.
    if (entry.leaf === ".zarray") node.kind = "array";
    if (parsed >= ZARR_METADATA_FILES) {
      node.unreadMetadata = true;
      continue;
    }
    parsed += 1;
    const read = readZarrMetadata(doc, entry);
    if (read.pending) return { status: "pending", reachedBytes: 0 };
    if (read.meta === null) {
      node.unreadMetadata = true;
      if (entry.leaf !== ".zattrs") {
        node.metadataPath = entry.path;
        node.metadataOffsetBits = entry.offsetBits;
      }
      continue;
    }
    if (entry.leaf === ".zattrs") {
      node.attributes = read.meta;
      continue;
    }
    node.meta = { ...read.meta, ...(node.meta ?? {}) };
    node.metadataPath = entry.path;
    node.metadataOffsetBits = entry.offsetBits;
    // v2 says which it is by which file it wrote; v3 by a field inside the one
    // file it writes, which also carries the attributes.
    if (entry.leaf === ".zarray") node.kind = "array";
    else if (entry.leaf === "zarr.json") {
      node.kind = read.meta["node_type"] === "array" ? "array" : "group";
      const attributes = jsonObject(read.meta["attributes"]);
      if (attributes !== null) node.attributes = attributes;
    }
  }
  // Every entry that is not metadata belongs to the innermost array above it.
  // What none of them claims is still in the archive and still gets counted:
  // a store whose metadata would not read has no arrays to own its chunks,
  // and a file zipped beside the store belongs to no array at all.
  const arrays = [...nodes.values()].filter((node) => node.kind === "array");
  let unowned = 0;
  for (const entry of entries) {
    if (ZARR_METADATA.has(entry.leaf) || entry.name.endsWith("/")) continue;
    let owner: ZarrNode | null = null;
    for (const array of arrays) {
      const inside = array.prefix === "" || entry.name.startsWith(`${array.prefix}/`);
      if (inside && (owner === null || array.prefix.length > owner.prefix.length)) owner = array;
    }
    if (owner !== null) owner.chunks.push(entry);
    else unowned += 1;
  }
  return { status: "ok", node: { nodes, unowned } };
}

/** The one line under the title: what the store is laid out as and how big.
 * The kind of store is the title and the Type column, so it is not repeated
 * here. */
function zarrSummary(
  ome: Record<string, unknown> | null,
  arrays: number,
  chunks: number,
  unowned: number,
  partial: boolean,
): string {
  const levels = omeLevels(ome);
  const axes = omeAxes(ome);
  const atLeast = partial ? "at least " : "";
  return [
    axes.length > 0 ? `axes ${axes.join(" ")}` : "",
    levels > 0 ? countText(levels, "resolution level") : "",
    countText(arrays, "array"),
    `${atLeast}${countText(chunks, "chunk")}`,
    unowned > 0 ? `${atLeast}${countText(unowned, "other entry")}` : "",
  ].filter(Boolean).join(" \u00b7 ");
}

function zarrOutline(
  doc: Doc,
  entries: readonly ZipEntry[],
  totalEntries: number,
  partial: boolean,
  expanded: ReadonlySet<string>,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> {
  const built = zarrNodes(doc, entries);
  if (built.status !== "ok") return built;
  const { nodes: store, unowned } = built.node;
  const rootMeta = store.get("")?.attributes ?? store.get("")?.meta ?? null;
  const rootOme = omeBlock(rootMeta);
  // A store written as a folder inside the archive puts everything one level
  // down, and that folder is the image rather than the archive.
  const shallowest = [...store.keys()].sort((a, b) => a.length - b.length)[0] ?? "";
  const top = shallowest.split("/")[0] ?? "";
  // An archive holding two stores side by side has no single folder to stand
  // for the whole of it, and folding one of them into the root would give two
  // rows the same place in the tree.
  const shared = top !== "" && [...store.keys()].every((prefix) => prefix === top || prefix.startsWith(`${top}/`));
  const base = store.has("") || !shared ? "" : top;
  const baseOme = rootOme ?? omeBlock(store.get(base)?.attributes ?? null);

  const nodes: LogicalNode[] = [];
  let chunkTotal = 0;
  for (const node of store.values()) chunkTotal += node.chunks.length;
  const arrays = [...store.values()].filter((node) => node.kind === "array").length;

  nodes.push({
    id: "/", parentId: null, label: baseOme === null ? "Store" : "Image", fullName: base === "" ? "/" : base,
    depth: 0, group: true, hasChildren: store.size > 0, sourcePath: [], sourceBits: 0,
    sourceText: formatOffset(0),
    value: zarrSummary(baseOme, arrays, chunkTotal, unowned, partial),
    type: baseOme === null ? "Zarr" : "OME-Zarr",
    logicalBytes: null, logicalApproximate: false,
    title: `${totalEntries.toLocaleString()} archive entries`,
  });

  const more: LogicalMore[] = [];
  const ordered = [...store.keys()].sort();
  for (const prefix of ordered) {
    const node = store.get(prefix);
    if (node === undefined) continue;
    const relative = base === "" ? prefix : prefix === base ? "" : prefix.slice(base.length + 1);
    const parts = relative === "" ? [] : relative.split("/");
    const id = `/${relative}`;
    const parentRelative = parts.slice(0, -1).join("/");
    const parentId = parts.length === 0 ? "/" : `/${parentRelative}`;
    const shape = numberList(node.meta?.["shape"]);
    const chunkShape = zarrChunkShape(node.meta);
    const element = zarrElement(node.meta);
    const codecs = zarrCodecs(node.meta);
    const times = (dims: readonly number[]): string => dims.map((d) => d.toLocaleString()).join(" \u00d7 ");
    // A scan that stopped short counted some of the chunks, not all of them,
    // and every count taken from it has to say so.
    const counted = `${partial ? "at least " : ""}${countText(node.chunks.length, "chunk")}`;
    const value = node.kind === "array"
      ? [
          shape.length > 0 ? times(shape) : "",
          chunkShape.length > 0 ? `${counted} of ${times(chunkShape)}` : counted,
          codecs.join(" then "),
          // Shape and element type are missing rather than absent, and a row
          // that just leaves them out looks like an array without them.
          node.meta === null && node.unreadMetadata ? "metadata not read" : "",
        ].filter(Boolean).join(" \u00b7 ")
      : node.unreadMetadata
        ? "metadata not read"
        : "group";
    const count = shape.length === 0 ? null : shapeCount(shape);
    const logicalBytes = count !== null && element.bytes !== null ? count * element.bytes : null;
    if (parts.length > 0 || prefix !== base) {
      nodes.push({
        id, parentId, label: parts.at(-1) ?? (prefix === "" ? "Root" : prefix),
        fullName: prefix === "" ? "/" : prefix, depth: parts.length,
        group: true, hasChildren: node.kind === "array" && node.chunks.length > 0,
        sourcePath: node.metadataPath, sourceBits: node.metadataOffsetBits,
        sourceText: formatOffset(node.metadataOffsetBits),
        value, type: node.kind === "array" ? element.name || "array" : "group",
        logicalBytes, logicalApproximate: false,
        title: `${prefix === "" ? "/" : prefix} \u00b7 ${node.kind}`,
      });
    }
    if (node.kind !== "array" || node.chunks.length === 0) continue;
    const chunksId = `${id === "/" ? "" : id}/chunks`;
    const stored = node.chunks.reduce((sum, chunk) => sum + chunk.compressedBytes, 0);
    // What one chunk holds once it is unpacked. The bytes in the archive are a
    // different number, and for a store worth packing this way a much smaller
    // one, so both are worth saying.
    const perChunk = shapeCount(chunkShape);
    const chunkBytes = perChunk !== null && element.bytes !== null ? perChunk * element.bytes : null;
    nodes.push({
      id: chunksId, parentId: parts.length === 0 ? "/" : id, label: "Chunks", fullName: `${prefix}/chunks`,
      depth: parts.length + 1, group: true, hasChildren: true,
      sourcePath: node.chunks[0]?.path ?? node.metadataPath,
      sourceBits: node.chunks[0]?.offsetBits ?? null,
      sourceText: "multiple",
      value: `${counted} \u00b7 ${formatBytes(stored)} in the archive`,
      type: "chunks",
      logicalBytes: chunkBytes === null ? null : chunkBytes * node.chunks.length,
      logicalApproximate: partial,
      title: `${counted} in the archive, ${formatBytes(stored)}`,
    });
    if (!expanded.has(chunksId)) continue;
    const limit = Math.min(shown.get(chunksId) ?? LOGICAL_PAGE, node.chunks.length);
    for (const chunk of node.chunks.slice(0, limit)) {
      const coordinate = chunkCoordinate(prefix, chunk.name);
      const packed = chunk.stored ? "" : chunk.compression;
      nodes.push({
        id: `${chunksId}/${chunk.path[1]}`, parentId: chunksId, label: coordinate, fullName: chunk.name,
        depth: parts.length + 2, group: false, hasChildren: false,
        sourcePath: chunk.path, sourceBits: chunk.offsetBits, sourceText: formatOffset(chunk.offsetBits),
        value: [`${formatBytes(chunk.compressedBytes)} in the archive`, packed].filter(Boolean).join(" \u00b7 "),
        type: "chunk", logicalBytes: chunkBytes, logicalApproximate: false,
        title: chunk.name,
      });
    }
    if (limit < node.chunks.length) {
      more.push({
        sectionId: chunksId,
        afterId: limit === 0 ? chunksId : `${chunksId}/${node.chunks[limit - 1]?.path[1]}`,
        count: node.chunks.length - limit, label: "chunks",
      });
    }
  }

  return {
    status: "ok",
    node: {
      format: "zarrzip",
      title: baseOme === null ? "Zarr store" : "OME-Zarr image",
      summary: zarrSummary(baseOme, arrays, chunkTotal, unowned, partial),
      nodes, total: nodes.length,
      sizeLabel: "Logical size",
      ...(more.length === 0 ? {} : { more }),
    },
  };
}

/** The archive as folders and files: what a ZIP holds when nothing more
 * specific is known about the entries. */
function plainZipOutline(
  entries: readonly ZipEntry[],
  totalEntries: number,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> {
  const limit = Math.min(shown.get("/entries") ?? LOGICAL_PAGE, entries.length);
  const folderNodes = new Map<string, LogicalNode>();
  const fileNodes: LogicalNode[] = [];
  let unpackedTotal = 0;
  for (const entry of entries.slice(0, limit)) {
    const parts = entry.name.split("/").filter(Boolean);
    const directory = entry.name.endsWith("/");
    let parentId = "/";
    const ancestors: string[] = [];
    for (let i = 0; i < Math.max(0, parts.length - (directory ? 0 : 1)); i++) {
      const label = parts[i] ?? "";
      const id = `${parentId === "/" ? "" : parentId}/${label}`;
      if (!folderNodes.has(id)) {
        folderNodes.set(id, {
          id, parentId, label, fullName: id, depth: i + 1, group: true, hasChildren: true,
          sourcePath: entry.path, sourceBits: null, sourceText: "multiple", value: "folder", type: "folder",
          logicalBytes: 0, logicalApproximate: false, title: id,
        });
      }
      parentId = id;
      ancestors.push(id);
    }
    if (directory) continue;
    unpackedTotal += entry.unpackedBytes;
    for (const id of ancestors) {
      const folder = folderNodes.get(id);
      if (folder !== undefined) folderNodes.set(id, { ...folder, logicalBytes: (folder.logicalBytes ?? 0) + entry.unpackedBytes });
    }
    fileNodes.push({
      id: `/entry/${entry.path[1]}`, parentId, label: parts.at(-1) ?? entry.name, fullName: entry.name,
      depth: parts.length, group: false, hasChildren: false, sourcePath: entry.path,
      sourceBits: entry.offsetBits, sourceText: formatOffset(entry.offsetBits),
      value: [`${formatBytes(entry.compressedBytes)} in the archive`, entry.stored ? "" : entry.compression]
        .filter(Boolean).join(" \u00b7 "),
      type: "file",
      logicalBytes: entry.unpackedBytes, logicalApproximate: false,
      title: `${entry.name} \u00b7 ${formatBytes(entry.compressedBytes)} in the archive, ${formatBytes(entry.unpackedBytes)} unpacked`,
    });
  }
  const root: LogicalNode = {
    id: "/", parentId: null, label: "Archive", fullName: "/", depth: 0, group: true, hasChildren: true,
    sourcePath: [], sourceBits: 0, sourceText: formatOffset(0), value: countText(totalEntries, "entry"),
    type: "ZIP", logicalBytes: unpackedTotal, logicalApproximate: limit < totalEntries,
    title: "ZIP archive",
  };
  const unordered = [...folderNodes.values(), ...fileNodes];
  const children = new Map<string, LogicalNode[]>();
  for (const node of unordered) {
    const parent = node.parentId ?? "/";
    const list = children.get(parent) ?? [];
    list.push(node);
    children.set(parent, list);
  }
  const nodes = [root];
  const append = (parent: string): void => {
    for (const node of children.get(parent) ?? []) {
      nodes.push(node);
      if (node.group) append(node.id);
    }
  };
  append("/");
  const remaining = totalEntries - limit;
  return {
    status: "ok",
    node: {
      format: "zip", title: "ZIP archive", summary: countText(totalEntries, "entry"), nodes,
      total: nodes.length + remaining, sizeLabel: "Unpacked size",
      ...(remaining === 0 ? {} : { more: [{ sectionId: "/entries", afterId: "/", count: remaining, label: "entries" }] }),
    },
  };
}

/** A ZIP is read as a Zarr store when its entries carry a store's metadata
 * keys, and as an ordinary archive when they do not. The names decide, not
 * the template, so a store the sniff did not catch still reads as one. */
function archiveOutline(
  doc: Doc,
  expanded: ReadonlySet<string>,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> {
  const walked = zipEntries(doc);
  if (walked.status !== "ok") return walked;
  const { entries, total, partial } = walked.node;
  const zarr = entries.some((entry) => ZARR_METADATA.has(entry.leaf));
  return zarr
    ? zarrOutline(doc, entries, total, partial, expanded, shown)
    : plainZipOutline(entries, total, shown);
}

function sqliteOutline(doc: Doc): TemplateReply<LogicalOutline> {
  const schemaReply = doc.templateNode([23, 6]);
  if (schemaReply.status !== "ok") return schemaReply;
  const pageSizeRaw = Number(nodeValue(doc, [1]));
  const pageSize = pageSizeRaw === 1 ? 65_536 : pageSizeRaw;
  const pageCount = Number.isFinite(pageSize) && pageSize > 0 ? Math.ceil(doc.lengthBytes / pageSize) : 0;
  const encoding = nodeValue(doc, [16])?.replace(/ \(\d+\)$/, "") ?? "text";
  const isSelf = doc.template === "self";
  const title = isSelf ? "SELF program database" : "SQLite schema";
  const root: LogicalNode = {
    id: "/", parentId: null, label: isSelf ? "Program database" : "Database", fullName: "/",
    depth: 0, group: true, hasChildren: true, sourcePath: [], sourceBits: 0, sourceText: formatOffset(0),
    value: `${pageCount.toLocaleString()} pages · ${formatBytes(pageSize)} page size · ${encoding}`,
    type: isSelf ? "SELF" : "SQLite", logicalBytes: null, logicalApproximate: false, title,
  };
  const groups = new Map<string, LogicalNode>();
  const entries: LogicalNode[] = [];
  for (let i = 0; i < schemaReply.node.child_count; i++) {
    const cellPath = [23, 6, i];
    const cell = doc.templateNode(cellPath);
    if (cell.status !== "ok") return cell;
    const record = [...cellPath, 2];
    const kind = (nodeValue(doc, [...record, 2]) ?? "object").toLowerCase();
    const plural = kind === "index" ? "indexes" : `${kind}s`;
    const pluralLabel = plural[0]?.toUpperCase() + plural.slice(1);
    const name = nodeValue(doc, [...record, 3]) ?? cell.node.name;
    const tableName = nodeValue(doc, [...record, 4]) ?? "";
    const rootPage = nodeValue(doc, [...record, 5]) ?? "0";
    const sql = nodeValue(doc, [...record, 6]) ?? "";
    const groupId = `/${kind}s`;
    const old = groups.get(groupId);
    groups.set(groupId, old === undefined ? {
      id: groupId, parentId: "/", label: pluralLabel, fullName: groupId,
      depth: 1, group: true, hasChildren: true, sourcePath: [23, 6], sourceBits: null, sourceText: "page 1",
      value: `1 ${kind}`, type: "schema group", logicalBytes: cell.node.size_bits / 8,
      logicalApproximate: false, title: `${kind} definitions in sqlite_schema`,
    } : {
      ...old,
      value: `${Number.parseInt(old.value, 10) + 1} ${plural}`,
      logicalBytes: (old.logicalBytes ?? 0) + cell.node.size_bits / 8,
    });
    const detail = [tableName !== name ? `on ${tableName}` : "", `root page ${rootPage}`, sql].filter(Boolean).join(" · ");
    entries.push({
      id: `${groupId}/${i}`, parentId: groupId, label: name, fullName: name, depth: 2, group: false,
      hasChildren: false, sourcePath: cellPath, sourceBits: cell.node.offset_bits,
      sourceText: formatOffset(cell.node.offset_bits), value: detail, type: kind,
      logicalBytes: cell.node.size_bits / 8, logicalApproximate: false,
      title: sql || `${kind} ${name}`,
    });
  }
  const nodes = [root];
  for (const group of groups.values()) {
    nodes.push(group, ...entries.filter((entry) => entry.parentId === group.id));
  }
  return {
    status: "ok",
    node: {
      format: isSelf ? "self" : "sqlite", title,
      summary: `${schemaReply.node.child_count.toLocaleString()} schema objects · ${pageCount.toLocaleString()} pages · ${formatBytes(pageSize)} page size`,
      nodes, total: nodes.length, sizeLabel: "Schema extent",
    },
  };
}

/** Adapters are intentionally independent of the view. GGUF, ZIP, SQLite,
 * RIFF and MP4 can add semantic nodes here without changing the table UI. */
const ADAPTERS: readonly Adapter[] = [
  { matches: (doc) => doc.template === "hdf5", read: (doc) => hdf5Outline(doc) },
  { matches: (doc) => doc.template === "gguf", read: ggufOutline },
  { matches: (doc) => doc.template === "zip" || doc.template === "zarrzip", read: archiveOutline },
  { matches: (doc) => doc.template === "sqlite" || doc.template === "self", read: (doc) => sqliteOutline(doc) },
];

export function hasLogicalOutline(doc: Doc): boolean {
  return ADAPTERS.some((adapter) => adapter.matches(doc));
}

export function logicalOutline(
  doc: Doc,
  expanded: ReadonlySet<string>,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> | null {
  return ADAPTERS.find((adapter) => adapter.matches(doc))?.read(doc, expanded, shown) ?? null;
}

export function logicalLength(node: LogicalNode): string {
  if (node.logicalBytes === null) return "—";
  return `${node.logicalApproximate ? "~" : ""}${formatBytes(node.logicalBytes)}`;
}
