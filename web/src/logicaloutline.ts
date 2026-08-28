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

function zipOutline(
  doc: Doc,
  _expanded: ReadonlySet<string>,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> {
  const recordsReply = doc.templateNode([0]);
  if (recordsReply.status !== "ok") return recordsReply;
  const locals: Array<{ path: number[]; node: import("./doc.js").TemplateNode }> = [];
  for (let i = 0; i < recordsReply.node.child_count; i++) {
    const signature = doc.templateNode([0, i, 0]);
    if (signature.status !== "ok") return signature;
    if (!signature.node.value.startsWith("local file")) continue;
    const record = doc.templateNode([0, i]);
    if (record.status !== "ok") return record;
    locals.push({ path: [0, i], node: record.node });
  }
  const limit = Math.min(shown.get("/entries") ?? LOGICAL_PAGE, locals.length);
  const folderNodes = new Map<string, LogicalNode>();
  const fileNodes: LogicalNode[] = [];
  let unpackedTotal = 0;
  for (const local of locals.slice(0, limit)) {
    const name = nodeValue(doc, [...local.path, 1, 10]) ?? local.node.name;
    const clean = name.replace(/\\/g, "/").replace(/^\/+/, "");
    const parts = clean.split("/").filter(Boolean);
    let parentId = "/";
    const ancestors: string[] = [];
    for (let i = 0; i < Math.max(0, parts.length - (clean.endsWith("/") ? 0 : 1)); i++) {
      const label = parts[i] ?? "";
      const id = `${parentId === "/" ? "" : parentId}/${label}`;
      if (!folderNodes.has(id)) {
        folderNodes.set(id, {
          id, parentId, label, fullName: id, depth: i + 1, group: true, hasChildren: true,
          sourcePath: local.path, sourceBits: null, sourceText: "multiple", value: "folder", type: "folder",
          logicalBytes: 0, logicalApproximate: false, title: id,
        });
      }
      parentId = id;
      ancestors.push(id);
    }
    if (clean.endsWith("/")) continue;
    const compressed = Number(nodeValue(doc, [...local.path, 1, 6]) ?? 0);
    const unpacked = Number(nodeValue(doc, [...local.path, 1, 7]) ?? 0);
    const compression = nodeValue(doc, [...local.path, 1, 2])?.replace(/ \(\d+\)$/, "") ?? "file";
    unpackedTotal += unpacked;
    for (const id of ancestors) {
      const folder = folderNodes.get(id);
      if (folder !== undefined) folderNodes.set(id, { ...folder, logicalBytes: (folder.logicalBytes ?? 0) + unpacked });
    }
    fileNodes.push({
      id: `/entry/${local.path[1]}`, parentId, label: parts.at(-1) ?? clean, fullName: clean,
      depth: parts.length, group: false, hasChildren: false, sourcePath: local.path,
      sourceBits: local.node.offset_bits, sourceText: formatOffset(local.node.offset_bits),
      value: `${formatBytes(compressed)} stored · ${compression}`, type: "file",
      logicalBytes: unpacked, logicalApproximate: false,
      title: `${clean} · ${formatBytes(compressed)} stored, ${formatBytes(unpacked)} unpacked`,
    });
  }
  const root: LogicalNode = {
    id: "/", parentId: null, label: "Archive", fullName: "/", depth: 0, group: true, hasChildren: true,
    sourcePath: [], sourceBits: 0, sourceText: formatOffset(0), value: `${locals.length.toLocaleString()} entries`,
    type: "ZIP", logicalBytes: unpackedTotal, logicalApproximate: limit < locals.length,
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
  const remaining = locals.length - limit;
  return {
    status: "ok",
    node: {
      format: "zip", title: "ZIP archive", summary: `${locals.length.toLocaleString()} entries`, nodes,
      total: nodes.length + remaining, sizeLabel: "Unpacked size",
      ...(remaining === 0 ? {} : { more: [{ sectionId: "/entries", afterId: "/", count: remaining, label: "entries" }] }),
    },
  };
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

const ELF_SECTION_KINDS = new Map<number, string>([
  [0, "null"], [1, "program data"], [2, "symbols"], [3, "strings"], [4, "relocations + addends"],
  [5, "hash"], [6, "dynamic linking"], [7, "notes"], [8, "uninitialized data"], [9, "relocations"],
  [11, "dynamic symbols"], [14, "initializers"], [15, "finalizers"], [16, "pre-initializers"], [17, "group"],
]);

const ELF_SYMBOL_KINDS = new Map<number, string>([
  [0, "symbol"], [1, "object"], [2, "function"], [3, "section"], [4, "file"], [5, "common"], [6, "TLS"], [10, "indirect function"],
]);

function enumName(value: string | null, fallback: string): string {
  return value?.replace(/ \([^)]*\)$/, "") || fallback;
}

function elfOutline(
  doc: Doc,
  expanded: ReadonlySet<string>,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> {
  const symbolLimit = expanded.has("/symbols") ? shown.get("/symbols") ?? LOGICAL_PAGE : 0;
  const reply = doc.elfContents(symbolLimit);
  if (reply.status !== "ok") return reply;
  const elf = reply.node;
  const header = [7];
  const bits = enumName(nodeValue(doc, [1]), "ELF");
  const endian = enumName(nodeValue(doc, [2]), "");
  const objectType = enumName(nodeValue(doc, [...header, 0]), "object");
  const machine = enumName(nodeValue(doc, [...header, 1]), "machine");
  const entry = Number(nodeValue(doc, [...header, 3]) ?? 0);
  const programHeaders = doc.templateNode([...header, 13, 0]);
  if (programHeaders.status !== "ok") return programHeaders;
  const nodes: LogicalNode[] = [{
    id: "/", parentId: null, label: "Program image", fullName: "/", depth: 0, group: true, hasChildren: true,
    sourcePath: [], sourceBits: 0, sourceText: formatOffset(0),
    value: [objectType, machine, bits, endian].filter(Boolean).join(" · "), type: doc.template === "bpf" ? "eBPF ELF" : "ELF",
    logicalBytes: doc.lengthBytes, logicalApproximate: false, title: `${machine} ${objectType}`,
  }];

  const addGroup = (id: string, label: string, count: number, value: string, sourcePath: number[]): void => {
    nodes.push({
      id, parentId: "/", label, fullName: id, depth: 1, group: true, hasChildren: count > 0,
      sourcePath, sourceBits: null, sourceText: count > 0 ? "table" : "—", value, type: "catalogue",
      logicalBytes: null, logicalApproximate: false, title: `${count.toLocaleString()} ${label.toLowerCase()}`,
    });
  };

  addGroup("/segments", "Segments", programHeaders.node.child_count, `${programHeaders.node.child_count.toLocaleString()} mapped regions`, [...header, 13]);
  for (let i = 0; i < programHeaders.node.child_count; i++) {
    const path = [...header, 13, 0, i];
    const segment = doc.templateNode(path);
    if (segment.status !== "ok") return segment;
    const wide = bits.startsWith("64");
    const offsetIndex = wide ? 2 : 1;
    const addressIndex = wide ? 3 : 2;
    const fileSizeIndex = wide ? 5 : 4;
    const memorySizeIndex = wide ? 6 : 5;
    const flagsIndex = wide ? 1 : 6;
    const fileSize = Number(nodeValue(doc, [...path, fileSizeIndex]) ?? 0);
    const memorySize = Number(nodeValue(doc, [...path, memorySizeIndex]) ?? 0);
    const offset = Number(nodeValue(doc, [...path, offsetIndex]) ?? 0);
    const address = nodeValue(doc, [...path, addressIndex]) ?? "0";
    const kind = enumName(nodeValue(doc, [...path, 0]), "segment");
    const flags = nodeValue(doc, [...path, flagsIndex]) ?? "";
    nodes.push({
      id: `/segments/${i}`, parentId: "/segments", label: `${kind} ${i + 1}`, fullName: `/segments/${i}`,
      depth: 2, group: false, hasChildren: false, sourcePath: path, sourceBits: segment.node.offset_bits,
      sourceText: formatOffset(segment.node.offset_bits),
      value: [`file ${formatOffset(offset * 8)}`, `virtual ${address}`, flags, memorySize !== fileSize ? `${formatBytes(fileSize)} file → ${formatBytes(memorySize)} memory` : ""].filter(Boolean).join(" · "),
      type: "segment", logicalBytes: memorySize, logicalApproximate: false, title: `${kind} segment`,
    });
  }

  addGroup("/sections", "Sections", elf.sections.length, `${elf.sections.length.toLocaleString()} linked sections`, [...header, 14]);
  for (let i = 0; i < elf.sections.length; i++) {
    const section = elf.sections[i];
    if (section === undefined) continue;
    const headerPath = section.path;
    const flags = nodeValue(doc, [...headerPath, 2]) ?? "";
    const kind = ELF_SECTION_KINDS.get(section.kind) ?? enumName(nodeValue(doc, [...headerPath, 1]), `type ${section.kind}`);
    nodes.push({
      id: `/sections/${i}`, parentId: "/sections", label: section.name || (i === 0 ? "Null section" : `Section ${i}`),
      fullName: section.name || `/sections/${i}`, depth: 2, group: false, hasChildren: false,
      sourcePath: [7, 15, i], sourceBits: section.offset * 8, sourceText: section.kind === 8 ? "memory only" : formatOffset(section.offset * 8),
      value: [kind, flags, section.address > 0 ? `virtual ${formatOffset(section.address * 8)}` : ""].filter(Boolean).join(" · "),
      type: "section", logicalBytes: section.size, logicalApproximate: false, title: section.name || `Section ${i}`,
    });
  }

  addGroup("/symbols", "Symbols", elf.symbol_total, `${elf.symbol_total.toLocaleString()} named and unnamed symbols`, [...header, 15]);
  for (let i = 0; i < elf.symbols.length; i++) {
    const symbol = elf.symbols[i];
    if (symbol === undefined) continue;
    const section = elf.sections[symbol.section];
    const kind = ELF_SYMBOL_KINDS.get(symbol.kind) ?? `type ${symbol.kind}`;
    nodes.push({
      id: `/symbols/${i}`, parentId: "/symbols", label: symbol.name || `(unnamed ${kind})`, fullName: symbol.name || `/symbols/${i}`,
      depth: 2, group: false, hasChildren: false, sourcePath: symbol.path,
      sourceBits: symbol.source_bits, sourceText: formatOffset(symbol.source_bits),
      value: [`value ${formatOffset(symbol.value * 8)}`, section?.name || "", symbol.size > 0 ? formatBytes(symbol.size) : ""].filter(Boolean).join(" · "),
      type: kind, logicalBytes: symbol.size, logicalApproximate: false, title: symbol.name || kind,
    });
  }
  const remaining = elf.symbol_total - elf.symbols.length;
  return {
    status: "ok",
    node: {
      format: doc.template ?? "elf", title: "ELF program image",
      summary: [machine, objectType, `${elf.sections.length.toLocaleString()} sections`, `${elf.symbol_total.toLocaleString()} symbols`, entry > 0 ? `entry ${formatOffset(entry * 8)}` : ""].filter(Boolean).join(" · "),
      nodes, total: nodes.length + remaining, sizeLabel: "Memory / data size",
      ...(remaining > 0 && expanded.has("/symbols") ? { more: [{ sectionId: "/symbols", afterId: elf.symbols.length === 0 ? "/symbols" : `/symbols/${elf.symbols.length - 1}`, count: remaining, label: "symbols" }] } : {}),
    },
  };
}

function isoOutline(
  doc: Doc,
  expanded: ReadonlySet<string>,
  shown: ReadonlyMap<string, number>,
): TemplateReply<LogicalOutline> {
  const volumeReply = doc.isoVolume();
  if (volumeReply.status !== "ok") return volumeReply;
  const volume = volumeReply.node;
  const volumeBytes = volume.blocks * volume.block_size;
  const title = volume.volume || "ISO 9660 volume";
  const nodes: LogicalNode[] = [{
    id: "/", parentId: null, label: "Disc image", fullName: "/", depth: 0, group: true, hasChildren: true,
    sourcePath: [], sourceBits: 0, sourceText: formatOffset(0),
    value: `${title} · ${volume.blocks.toLocaleString()} blocks · ${formatBytes(volume.block_size)} blocks`,
    type: "ISO 9660", logicalBytes: volumeBytes, logicalApproximate: false, title,
  }, {
    id: "/volume", parentId: "/", label: "Volume information", fullName: "/volume", depth: 1,
    group: false, hasChildren: false, sourcePath: volume.descriptor_path, sourceBits: 16 * 2048 * 8,
    sourceText: "sector 16", value: title, type: "primary volume", logicalBytes: 2048,
    logicalApproximate: false, title: `${title} primary volume descriptor`,
  }, {
    id: "/files", parentId: "/", label: "Files", fullName: "/files", depth: 1, group: true, hasChildren: true,
    sourcePath: [...volume.descriptor_path, 14], sourceBits: volume.root_extent * volume.block_size * 8,
    sourceText: formatOffset(volume.root_extent * volume.block_size * 8), value: "root directory", type: "directory",
    logicalBytes: volume.root_size, logicalApproximate: false, title: "Root directory",
  }];
  const more: LogicalMore[] = [];
  let omitted = 0;
  const visited = new Set<string>();
  const addDirectory = (id: string, extent: number, size: number, depth: number): TemplateReply<never> | null => {
    if (!expanded.has(id)) return null;
    const visit = `${extent}:${size}`;
    if (visited.has(visit)) return null;
    visited.add(visit);
    const limit = shown.get(id) ?? LOGICAL_PAGE;
    const reply = doc.isoDirectory(extent, size, volume.block_size, limit);
    if (reply.status !== "ok") return reply;
    const children: Array<{ id: string; extent: number; size: number; depth: number }> = [];
    for (let i = 0; i < reply.node.entries.length; i++) {
      const entry = reply.node.entries[i];
      if (entry === undefined) continue;
      const childId = `${id}/${encodeURIComponent(entry.name || `entry-${i}`)}~${i}`;
      const dataBits = entry.extent * volume.block_size * 8;
      nodes.push({
        id: childId, parentId: id, label: entry.name || `(entry ${i + 1})`, fullName: childId.slice(6),
        depth, group: entry.directory, hasChildren: entry.directory,
        sourcePath: [...volume.descriptor_path, 14], sourceBits: dataBits, sourceText: formatOffset(dataBits),
        value: entry.directory ? "directory" : "file data", type: entry.directory ? "directory" : "file",
        logicalBytes: entry.size, logicalApproximate: false,
        title: `${entry.name || `Entry ${i + 1}`} · directory record at ${formatOffset(entry.source_bits)}`,
      });
      if (entry.directory) children.push({ id: childId, extent: entry.extent, size: entry.size, depth: depth + 1 });
    }
    const remaining = reply.node.total - reply.node.entries.length;
    omitted += remaining;
    if (remaining > 0) {
      more.push({
        sectionId: id,
        afterId: reply.node.entries.length === 0 ? id : `${id}/${encodeURIComponent(reply.node.entries.at(-1)?.name || `entry-${reply.node.entries.length - 1}`)}~${reply.node.entries.length - 1}`,
        count: remaining,
        label: "entries",
      });
    }
    for (const child of children) {
      const pending = addDirectory(child.id, child.extent, child.size, child.depth);
      if (pending !== null) return pending;
    }
    return null;
  };
  const pending = addDirectory("/files", volume.root_extent, volume.root_size, 2);
  if (pending !== null) return pending;
  return {
    status: "ok",
    node: {
      format: "iso9660", title: "ISO 9660 filesystem",
      summary: `${title} · ${volume.blocks.toLocaleString()} blocks · ${formatBytes(volumeBytes)}`,
      nodes, total: nodes.length + omitted, sizeLabel: "Data size", ...(more.length > 0 ? { more } : {}),
    },
  };
}

/** Adapters are intentionally independent of the view. GGUF, ZIP, SQLite,
 * RIFF and MP4 can add semantic nodes here without changing the table UI. */
const ADAPTERS: readonly Adapter[] = [
  { matches: (doc) => doc.template === "hdf5", read: (doc) => hdf5Outline(doc) },
  { matches: (doc) => doc.template === "gguf", read: ggufOutline },
  { matches: (doc) => doc.template === "zip", read: zipOutline },
  { matches: (doc) => doc.template === "sqlite" || doc.template === "self", read: (doc) => sqliteOutline(doc) },
  { matches: (doc) => doc.template === "elf" || doc.template === "bpf", read: elfOutline },
  { matches: (doc) => doc.template === "iso9660", read: isoOutline },
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
