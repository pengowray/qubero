/**
 * A treemap: a box divided so that every rectangle's area is what it stands
 * for, nested so that a rectangle inside another is a part of it.
 *
 * The one picture that answers "what is this file made of" without reading a
 * byte of it. The map beside it says where things are; this says how much of
 * the file they are, which is the question a map of a gigabyte cannot answer
 * because the small things vanish into a pixel and the big things look like
 * everything else.
 *
 * The layout is `d3-hierarchy`'s treemap, squarified where the order of the
 * boxes carries nothing and binary where it does, so a caller whose children
 * are a sequence (the byte values 00 to FF) gets them laid out in that
 * sequence instead of largest first. What is written here is everything
 * around it: which nodes are worth descending into at a given size, what a
 * rectangle small enough to be a sliver becomes instead, where the names go,
 * and how the drawn boxes carry a click and a hover back to the caller.
 *
 * Names are drawn in a layer over the boxes rather than inside them. A frame
 * that kept a strip along its top for its own name would be taking that strip
 * out of its children's area, so the children would draw smaller than they
 * are and nothing on the picture would say so. Over the top the name costs no
 * area at all, and the rule for two names in one place is the useful one: the
 * outer name is bigger and wins, which is what tells a reader which box they
 * are inside.
 *
 * The widget knows nothing about files. It takes a tree of values with colours
 * already chosen and gives back an element; what the values mean is the
 * caller's business, which is what lets the same widget draw the file's
 * structure, its field kinds, its byte values and its bits.
 */

import { hierarchy, treemap, treemapBinary, treemapSquarify, type HierarchyRectangularNode } from "d3-hierarchy";

/** One box. `value` is in whatever unit the caller is counting, and only its
 *  share of the total matters here. A node with children is drawn as a frame
 *  around them when there is room and as a solid box when there is not, so a
 *  caller may hand over a whole tree and let the size decide how much of it
 *  appears. */
export type TreeNode = {
  /** Stable across redraws, so a redraw of the same tree keeps the pointer on
   *  the same box. Unique among siblings is enough; the widget makes the rest
   *  of the identity out of the path to it. */
  readonly key: string;
  readonly name: string;
  readonly value: number;
  /** Any CSS colour. The widget never picks one: what a box means is the
   *  caller's vocabulary, and two callers with two vocabularies would
   *  otherwise end up with one palette between them. */
  readonly color: string;
  /** A class that sets `--field-color`, for a caller drawing in a palette the
   *  stylesheet already keeps in both themes. Wins over `color` when given,
   *  which is how the kinds reuse the colours the hex view paints them. */
  readonly colorClass?: string;
  /** The second line of the box's tooltip: what it is, beside how much. */
  readonly detail?: string;
  /** Where in the file, for a caller that wants a click to go there. */
  readonly range?: { readonly offsetBits: number; readonly sizeBits: number };
  /** Which node of the template this box is, where it is one. What a reader
   *  opens a box into, and not derivable from the keys: a box's key is only
   *  unique among its siblings and a listing's key is not a path at all. */
  readonly path?: readonly number[];
  /** Which elements of its parent this box stands for, where it stands for a
   *  run of them rather than one node of the template. A long array is drawn
   *  as a handful of index ranges, and opening one has to say which. */
  readonly span?: { readonly from: number; readonly to: number; readonly offsetBits: number; readonly sizeBits: number };
  /** True when there is more inside this box than the tree carries, so a
   *  reader who opens it gets something they cannot already see. A box with
   *  drawn children can still be worth opening; a byte value never is. */
  readonly openable?: boolean;
  readonly children?: readonly TreeNode[];
};

/** What a click and a double click were on. The node itself rather than its
 *  key, since every caller wants the range and most want the name too. */
export type TreemapPick = {
  readonly node: TreeNode;
  /** Root first, this node last: the way back out, for a trail. */
  readonly trail: readonly TreeNode[];
  /** The identity the widget knows the box by, for lighting it. */
  readonly key: string;
};

export type TreemapOptions = {
  readonly width: number;
  readonly height: number;
  /** What one box says when the pointer rests on it. Given the node and its
   *  share of the whole box, since a share is what the picture is made of and
   *  the node alone cannot know it. */
  readonly title: (node: TreeNode, share: number) => string;
  /** What a pooled box is called, and what it says on hover. */
  readonly poolName: (n: number) => string;
  readonly poolDetail: (n: number) => string;
  /** Boxes the layout would give fewer square pixels than this are pooled into
   *  one rather than drawn as slivers nobody can point at. Null pools nothing.
   *
   *  A pixel count and not a share of the file: what makes a box unpointable
   *  is how small it comes out, and a child of a small parent is small however
   *  large its share of the whole happens to be. */
  readonly poolUnder?: number | null;
  /** Lay the children out in the order the caller gave them instead of
   *  largest first. For a sequence, which the byte values are: 0x00 belongs at
   *  the top left and 0xFF at the bottom right, and a picture that put the
   *  commonest value at the top left would be sorted by the one thing the
   *  reader can already see. */
  readonly ordered?: boolean;
  /** The box drawn lit, by the identity a pick gave back. */
  readonly selected?: string | null;
};

/**
 * A box has to be about this big before its children are worth drawing inside
 * it. Below it the gaps eat the area the children were supposed to show, and
 * a reader sees texture rather than parts.
 */
const NEST_AREA = 700;
/** The border a frame keeps around its children.
 *
 *  Even on all four sides, so nesting costs a box the same share of its width
 *  as of its height and no direction is quietly squeezed. Wide enough to press:
 *  a frame's children tile the whole of it, so this rim and the frame's own
 *  name are the only parts of it a pointer can reach, and at two pixels it was
 *  a frame nobody could select. */
const FRAME_PAD = 5;
/** A long name is written over its box once this much of it fits, and cut off
 *  with an ellipsis. A short one has to fit whole or not be drawn at all:
 *  `program_header_off…` is still worth reading and `0x8…` is not, because a
 *  cut-off byte value is a different byte value. */
const MIN_CHARS = 9;
/** No tree is descended past this, however much room there is. A file whose
 *  structure runs forty levels deep would otherwise spend the whole box on
 *  frames. */
const MAX_DEPTH = 7;

/** How big a name is drawn, by how deep the box is. The outermost boxes are
 *  the ones a reader is orienting by, so their names are the ones worth
 *  reading across the room; a name four levels in is a detail and is drawn
 *  like one. Past the end of the list every level is the last size. */
const LABEL_SIZES = [17, 14, 12, 11, 10];
/** How wide one character is at one size, near enough to place a name by. The
 *  face is the monospace stack, so this is a ratio and not a guess. */
const CHAR_RATIO = 0.6;
/** Room around a name inside its plate. */
const LABEL_PAD_X = 4;
const LABEL_PAD_Y = 1;

/** How much of a frame is its own rim: enough to press, and nothing for a box
 *  with no children to keep anything away from. */
function rim(d: HierarchyRectangularNode<TreeNode>): number {
  return d.children === undefined || d.children.length === 0 ? 1 : FRAME_PAD;
}

/** The same node with nothing inside it. Not `children: undefined`, which
 *  under `exactOptionalPropertyTypes` is a different type from a node that
 *  never had the property. */
function leafOf(node: TreeNode): TreeNode {
  const { children: _inside, ...rest } = node;
  return rest;
}

/**
 * How much of the box a node will get, near enough to decide by. The real
 * areas come out of the layout, and the layout cannot run until the tree has
 * been decided, so this estimate is what breaks the circle: a node's share of
 * its parent's value is its share of its parent's area.
 */
function pruned(node: TreeNode, area: number, depth: number, pool: Pool): TreeNode {
  const kids = node.children ?? [];
  const roomy = area >= NEST_AREA && depth < MAX_DEPTH;
  if (kids.length === 0 || !roomy) return leafOf(node);
  const total = kids.reduce((n, k) => n + Math.max(0, k.value), 0);
  if (total <= 0) return leafOf(node);
  // The border the frame keeps is not the children's to divide. Taken off
  // both sides of a square of the same area, which is what the layout will
  // roughly hand back.
  const side = Math.max(0, Math.sqrt(area) - 2 * FRAME_PAD);
  const inner = side * side;
  const kept: TreeNode[] = [];
  let pooledValue = 0;
  let pooledCount = 0;
  for (const kid of kids) {
    const share = Math.max(0, kid.value) / total;
    const got = inner * share;
    if (pool.under !== null && got < pool.under) {
      pooledValue += Math.max(0, kid.value);
      pooledCount += 1;
      continue;
    }
    kept.push(pruned(kid, got, depth + 1, pool));
  }
  if (pooledValue > 0) {
    kept.push({
      key: `${node.key} pool`,
      name: pool.name(pooledCount),
      value: pooledValue,
      color: node.color,
      colorClass: "tm-pooled",
      detail: pool.detail(pooledCount),
    });
  }
  // Everything pooled is nothing shown. A frame holding one box the size of
  // itself is a box with a second name written over it and a border stealing
  // two pixels, so the node goes back to being what it looks like: solid, and
  // still openable, which is where the parts a reader wants are.
  if (kept.length === 0 || (kept.length === 1 && pooledCount > 0)) return leafOf(node);
  return { ...node, children: kept };
}

type Pool = {
  readonly under: number | null;
  readonly name: (n: number) => string;
  readonly detail: (n: number) => string;
};

/** What the widget hands back: the element to put on the page, and the boxes
 *  it drew, so a caller can light one without going through the DOM. */
export type Treemap = {
  readonly el: HTMLElement;
  /** Every drawn box by its identity, innermost last. */
  readonly boxes: ReadonlyMap<string, HTMLElement>;
};

/**
 * Draw `root` into a box of the given size.
 *
 * The element positions its boxes absolutely inside itself, so the caller owns
 * the size and the widget never measures anything: a treemap that read its own
 * width would be a treemap that drew itself wrong the first time, before the
 * layout it is inside had settled.
 */
export function drawTreemap(root: TreeNode, opts: TreemapOptions): Treemap {
  const el = document.createElement("div");
  el.className = "tm";
  el.style.width = `${opts.width}px`;
  el.style.height = `${opts.height}px`;
  const boxes = new Map<string, HTMLElement>();
  const whole = Math.max(0, root.value);
  if (whole <= 0 || opts.width < 8 || opts.height < 8) return { el, boxes };

  const shaped = pruned(root, opts.width * opts.height, 0, {
    under: opts.poolUnder ?? null,
    name: opts.poolName,
    detail: opts.poolDetail,
  });

  const tree = hierarchy<TreeNode>(shaped, (d) => d.children as TreeNode[] | undefined).sum((d) =>
    d.children === undefined || d.children.length === 0 ? Math.max(0, d.value) : 0,
  );
  // Sorting is what puts the biggest box at the top left, and it is right
  // whenever the order of the children says nothing. Where the order is the
  // point, both the sort and the squarified tiling have to go: squarify walks
  // the children in order and would still scatter a sequence, where the binary
  // tiling keeps it and stays near enough to square to compare areas by eye.
  if (opts.ordered !== true) tree.sort((a, b) => (b.value ?? 0) - (a.value ?? 0));
  const laid = treemap<TreeNode>()
    .size([opts.width, opts.height])
    .tile(opts.ordered === true ? treemapBinary : treemapSquarify)
    .paddingOuter(1)
    .paddingInner(1)
    // The same on every side, which takes four accessors: `paddingTop` alone
    // pads the top alone, so a frame meant to have a rim a pointer could reach
    // had one along its top edge and a hairline everywhere else.
    //
    // A frame keeps a border of its own colour and takes nothing else: the
    // strip a treemap usually reserves along the top for its name is area
    // stolen from the children, who then draw smaller than they are with
    // nothing on the picture to say so. The names go over the top instead.
    .paddingTop(rim)
    .paddingRight(rim)
    .paddingBottom(rim)
    .paddingLeft(rim)
    .round(true)(tree);

  // The root itself is the widget's own box, so it is not drawn; everything
  // under it is, parents before children, which is what puts a child on top of
  // the frame it sits in without a stacking context per level.
  const named: HierarchyRectangularNode<TreeNode>[] = [];
  for (const node of laid.descendants()) {
    if (node.depth === 0) continue;
    const key = identity(node);
    const box = drawBox(node, key, opts.title(node.data, whole === 0 ? 0 : (node.value ?? 0) / whole));
    if (box === null) continue;
    if (key === opts.selected) box.classList.add("is-on");
    boxes.set(key, box);
    el.append(box);
    named.push(node);
  }
  el.append(labelLayer(named, boxes));
  return { el, boxes };
}

/**
 * The names, in one layer over the boxes.
 *
 * Placed outermost first, and a name that would land on one already placed is
 * left off. That order is the whole rule: the box a reader is orienting by is
 * the one whose name they need, and the names inside it are the ones they can
 * get from the tooltip. It is also why the outer names may be drawn large
 * without crowding anything out permanently, since opening that box makes it
 * the outermost and its children's names the large ones.
 */
function labelLayer(nodes: readonly HierarchyRectangularNode<TreeNode>[], boxes: ReadonlyMap<string, HTMLElement>): HTMLElement {
  const layer = document.createElement("div");
  layer.className = "tm-names";
  const placed: Rect[] = [];
  for (const node of nodes) {
    const w = node.x1 - node.x0;
    const h = node.y1 - node.y0;
    const text = node.data.name;
    // The size the depth asks for, and then smaller until it fits. Asking once
    // and giving up left a hundred-pixel box unlabelled because its name did
    // not fit at seventeen point, and an unlabelled box is a box a reader
    // cannot name, cannot look up and does not know they can press.
    const fit = fits(text, node.depth, w, h);
    if (fit === null) continue;
    const { size, sideways, wanted, lineH } = fit;
    const along = sideways ? h : w;
    const width = Math.min(wanted, along - 2);
    // The rectangle the name will cover, in the layer's own axes, so two names
    // are compared the same way whichever direction each of them runs.
    const want = sideways
      ? { x0: node.x0 + 1, y0: node.y0 + 1, x1: node.x0 + 1 + lineH, y1: node.y0 + 1 + width }
      : { x0: node.x0 + 1, y0: node.y0 + 1, x1: node.x0 + 1 + width, y1: node.y0 + 1 + lineH };
    const rect = free(want, placed, node.y1 - 1, sideways);
    if (rect === null) continue;
    placed.push(rect);
    const label = document.createElement("span");
    label.className = sideways ? "tm-name tm-name-down" : "tm-name";
    label.textContent = text;
    label.style.left = `${sideways ? rect.x0 : node.x0 + 1}px`;
    label.style.top = `${sideways ? node.y0 + 1 : rect.y0}px`;
    label.style.maxWidth = `${along - 2}px`;
    label.style.fontSize = `${size}px`;
    label.style.lineHeight = `${size + 2}px`;
    // The name of a frame is drawn over its children, so a press on it has to
    // reach the frame rather than whichever child happens to be under it.
    const key = identity(node);
    label.dataset["key"] = key;
    const box = boxes.get(key);
    if (box !== undefined) label.title = box.title;
    layer.append(label);
  }
  return layer;
}

type Rect = { x0: number; y0: number; x1: number; y1: number };

/** A name that fits: how big, which way up, and the two widths the caller
 *  places it by. */
type Fit = { size: number; sideways: boolean; wanted: number; lineH: number };

/**
 * The biggest size a name fits at, upright if it can be and turned on its side
 * if it cannot.
 *
 * The depth picks where to start, because a box a reader is orienting by wants
 * a name they can read across the room. It does not pick where to stop: a box
 * a hundred pixels wide holding `field-number` does not fit at seventeen point
 * and fits perfectly at eleven, and refusing to shrink left the outermost
 * boxes, the ones most worth naming, as the only unlabelled ones on the map.
 */
function fits(text: string, depth: number, w: number, h: number): Fit | null {
  const start = Math.min(depth - 1, LABEL_SIZES.length - 1);
  for (let i = start; i < LABEL_SIZES.length; i++) {
    const size = LABEL_SIZES[i] ?? 10;
    const lineH = size + 2 * LABEL_PAD_Y + 2;
    // Room for the plate and the pixel of box either side of it. Without the
    // slack a name whose box was two pixels short of holding it was drawn and
    // then cut off, which for `0x8d` means showing a different byte value.
    const need = Math.min(text.length, MIN_CHARS) * size * CHAR_RATIO + 2 * LABEL_PAD_X + 3;
    const wanted = text.length * size * CHAR_RATIO + 2 * LABEL_PAD_X;
    if (h >= lineH && w >= need) return { size, sideways: false, wanted, lineH };
    // A tall narrow box has the room, only the other way up. Turning the name
    // on its side is what fills the column down the side of a treemap that
    // would otherwise be a stack of unlabelled slivers, and a reader tilting
    // their head is still a reader who can tell which box is which.
    if (w >= lineH && h >= need) return { size, sideways: true, wanted, lineH };
  }
  return null;
}

/**
 * Somewhere in the box for a name, given the names already down.
 *
 * A name wants the top left of its box, and so does the name of every frame
 * around it: a chain of nested frames all start within a pixel or two of the
 * same corner. Refusing every name but the outermost would hide exactly the
 * chain a reader needs, which is what says how the box they have opened sits
 * inside the one they came from, so a name that lands on one already down is
 * dropped a line and tried again. A few lines in it has run out of box, and
 * then it is genuinely not drawn.
 */
function free(want: Rect, placed: readonly Rect[], bottom: number, sideways = false): Rect | null {
  const height = want.y1 - want.y0;
  const width = want.x1 - want.x0;
  let rect = want;
  for (let tries = 0; tries < STACKED_NAMES; tries++) {
    const hit = placed.find((p) => p.x0 < rect.x1 && rect.x0 < p.x1 && p.y0 < rect.y1 && rect.y0 < p.y1);
    // A name running down the side steps sideways out of the way rather than
    // down, since down is the direction it is already using.
    if (hit === undefined) return sideways || rect.y1 <= bottom ? rect : null;
    rect = sideways
      ? { ...rect, x0: hit.x1, x1: hit.x1 + width }
      : { ...rect, y0: hit.y1, y1: hit.y1 + height };
  }
  return null;
}

/** How many names may stack down one corner. Four levels of containment is
 *  more than a reader is holding at once, and past that the names have walked
 *  far enough down the box to stop reading as its own. */
const STACKED_NAMES = 4;

/** A node's identity among all the drawn boxes: the keys from the root down,
 *  so two children of different parents with the same key stay apart. */
function identity(node: HierarchyRectangularNode<TreeNode>): string {
  return node
    .ancestors()
    .reverse()
    .map((a) => a.data.key)
    .join("/");
}

function drawBox(node: HierarchyRectangularNode<TreeNode>, key: string, title: string): HTMLElement | null {
  const w = node.x1 - node.x0;
  const h = node.y1 - node.y0;
  // A box the layout gave nothing to is not drawn at all. Rounding leaves
  // these behind wherever a value was a rounding error of the whole.
  if (w < 1 || h < 1) return null;
  const frame = node.children !== undefined && node.children.length > 0;
  const box = document.createElement("div");
  const paint = node.data.colorClass;
  // Openable when the caller says there is more inside than it handed over, and
  // whenever there are children drawn: both open, so both say so with the
  // pointer rather than one of them looking inert.
  const open = node.data.openable === true || frame;
  box.className = `tm-box${frame ? " tm-frame" : ""}${open ? " tm-open" : ""}${paint === undefined ? "" : ` ${paint}`}`;
  box.style.left = `${node.x0}px`;
  box.style.top = `${node.y0}px`;
  box.style.width = `${w}px`;
  box.style.height = `${h}px`;
  box.style.background = paint === undefined ? node.data.color : "var(--field-color)";
  box.dataset["key"] = key;
  box.title = title;
  return box;
}

/** Which node a click landed on, given the element the event reached. Returns
 *  the identity the caller can look the node up by; the caller keeps the tree,
 *  since it built it. */
export function boxAt(target: EventTarget | null): string | null {
  if (!(target instanceof HTMLElement)) return null;
  const box = target.closest<HTMLElement>("[data-key]");
  return box?.dataset["key"] ?? null;
}

/** Walk a tree to the node an identity names, and the nodes above it. Null
 *  when the tree has changed under the identity, which is what a click on a
 *  box drawn before a redraw looks like. */
export function nodeAt(root: TreeNode, identityPath: string): TreemapPick | null {
  const keys = identityPath.split("/");
  if (keys[0] !== root.key) return null;
  const trail: TreeNode[] = [root];
  let at = root;
  for (const key of keys.slice(1)) {
    const next = (at.children ?? []).find((k) => k.key === key);
    if (next === undefined) return null;
    trail.push(next);
    at = next;
  }
  return { node: at, trail, key: identityPath };
}
