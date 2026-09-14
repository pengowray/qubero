// The original irregular silhouette, cut from a solid with broad unequal
// planes. The front keeps the long skewed facets and small chipped shoulders;
// the back has its own cuts instead of mirroring a relief at a flat rim.
type Vertex = readonly [number, number, number];
type Plane = readonly [number, number, number, number];
const contour = [
  [285, 10], [140, 100], [65, 160], [43, 210], [12, 480],
  [20, 540], [59, 575], [163, 760], [194, 808], [401, 685],
  [426, 625], [449, 530], [465, 281], [433, 235], [355, 108],
] as const;
// Plane inequalities: ax + by + cz <= d. Front cuts preserve the drawing's
// broad upper face, slanted central face, and narrow bevels along the left.
const cuts: Plane[] = [
  [-0.12, -0.03, 1, 113.9],
  [-2, -0.08, 1, -3],
  [1.1, 0.1, 1, 528],
  [1.1, -1.2, 1, 215.72],
  [0.02, -1, 1, -125],
  [-0.3355, -0.30735, 1, 1.85],
  [-2.3608, -0.85058, 1, -339.47],
  [0.752639, 1, 1, 979.61],
  [1.1, 1.1, 1, 1137],
  [-1.3, 0.97, 1, 619.28],
  [-0.26, 0.95, 1, 742.56],
  [1.8, 0.12, 1, 856],
  // The rear's long face tapers between oblique shoulder and base cuts,
  // avoiding a rectangular panel surrounded by matching bevels.
  [0.30, -0.08, -1, 215],
  [-0.25, -0.65, -1, 50],
  [0.2, 0.8, -1, 660],
  [-1.3, -0.05, -1, -44.5],
  [1.1, 0.12, -1, 547.7],
  [-0.54, -0.0175, -1, 74.75],
  [-0.08, -0.34, -1, 98],
  [0.65, 0.50, -1, 635],
];
// Only supporting edges define the silhouette; a small inward kink in the
// drawing must not slice off a distant tip when extended as a cutting plane.
const sortedContour = [...contour].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
const halfContour = (vertices: readonly (readonly [number, number])[]): (readonly [number, number])[] => {
  const hull: (readonly [number, number])[] = [];
  for (const p of vertices) {
    while (hull.length > 1) {
      const a = hull[hull.length - 2]!, b = hull[hull.length - 1]!;
      if ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) > 0) break;
      hull.pop();
    }
    hull.push(p);
  }
  hull.pop();
  return hull;
};
const silhouette = [...halfContour(sortedContour), ...halfContour([...sortedContour].reverse())].reverse();
for (let i = 0; i < silhouette.length; i++) {
  const [x, y] = silhouette[i]!;
  const [nextX, nextY] = silhouette[(i + 1) % silhouette.length]!;
  const a = y - nextY, b = nextX - x;
  cuts.push([a, b, 0, a * x + b * y]);
}
const dot = (a: readonly number[], b: readonly number[]): number =>
  a[0]! * b[0]! + a[1]! * b[1]! + a[2]! * b[2]!;
const cross3 = (a: readonly number[], b: readonly number[]): Vertex =>
  [a[1]! * b[2]! - a[2]! * b[1]!, a[2]! * b[0]! - a[0]! * b[2]!, a[0]! * b[1]! - a[1]! * b[0]!];
// Intersect the cuts once. Every resulting facet is planar, so the bevels
// connect in depth without triangulation lines or a front/back joining ring.
const points: Vertex[] = [];
for (let i = 0; i < cuts.length; i++) {
  for (let j = i + 1; j < cuts.length; j++) {
    for (let k = j + 1; k < cuts.length; k++) {
      const a = cuts[i]!, b = cuts[j]!, c = cuts[k]!;
      const bc = cross3(b, c), ca = cross3(c, a), ab = cross3(a, b);
      const determinant = dot(a, bc);
      if (Math.abs(determinant) < 1e-8) continue;
      const coordinate = (axis: number): number =>
        (a[3] * bc[axis]! + b[3] * ca[axis]! + c[3] * ab[axis]!) / determinant;
      const p: Vertex = [coordinate(0), coordinate(1), coordinate(2)];
      if (cuts.some(plane => dot(plane, p) - plane[3] > 1e-5)) continue;
      if (!points.some(v => Math.hypot(v[0] - p[0], v[1] - p[1], v[2] - p[2]) < 1e-4)) points.push(p);
    }
  }
}
const facets = cuts.flatMap(plane => {
  const face = points.flatMap((p, i) => Math.abs(dot(plane, p) - plane[3]) < 1e-5 ? [i] : []);
  if (face.length < 3) return [];
  const center = [0, 1, 2].map(axis => face.reduce((sum, i) => sum + points[i]![axis]!, 0) / face.length);
  const u = cross3(plane, Math.abs(plane[0]) < Math.abs(plane[1]) ? [1, 0, 0] : [0, 1, 0]);
  const length = Math.hypot(...u);
  const unit = u.map(n => n / length);
  const v = cross3(plane, unit);
  const vLength = Math.hypot(...v);
  const angle = (i: number): number => {
    const offset = points[i]!.map((n, axis) => n - center[axis]!);
    return Math.atan2(dot(v, offset) / vLength, dot(unit, offset));
  };
  face.sort((a, b) => angle(a) - angle(b));
  return [face];
});
const NS = "http://www.w3.org/2000/svg";
const TURN = 2400;

// A spring pulling the mark back to rest, in radians per second per radian,
// and the drag that takes the energy out of it. Stiffness sets how long the
// return takes and damping sets how much of it is overshoot: three tenths of
// critical gives two visible bounces, which is a spring rather than a slump,
// and settles inside a second however far it was turned.
const STIFFNESS = 120;
const DAMPING = 6.6;

// A frame's worth of physics, at most. A tab in the background hands back one
// enormous step when it wakes, which a spring integrated in one go answers by
// flinging the mark to infinity.
const LONGEST_STEP = 1 / 30;

// How finely the spring is integrated, whatever the frame rate. A hard flick
// is several turns a second, and Euler steps that big wander off.
const SUB_STEP = 1 / 240;

// Movement a press has to stay inside to still count as a click, in pixels.
// A press that wandered further was aiming to turn the mark, and the spin a
// click gives it would fight the spring on the way out.
const A_CLICK = 4;

// How recent a pointer sample has to be to count towards the speed a release
// hands the spring, in milliseconds. Long enough to average out the jitter of
// one report, short enough that stopping dead before letting go means
// stopping dead.
const RECENT = 80;

export class Crystal {
  readonly el = document.createElement("button");
  private readonly svg = document.createElementNS(NS, "svg");
  private readonly outline = document.createElementNS(NS, "path");
  private readonly paths = Array.from({ length: facets.length }, () => document.createElementNS(NS, "path"));
  private frame = 0;
  private began = 0;
  private turns = 1;
  private busy = false;
  private readonly reduced = matchMedia("(prefers-reduced-motion: reduce)");
  // Where a drag has turned the mark to, and how fast it was going when it was
  // let go. `held` is the pointer doing the turning, or null when none is.
  private angle = 0;
  private speed = 0;
  private held: number | null = null;
  private from = 0;
  private radiansPerPx = 0;
  private wandered = 0;
  private samples: { at: number; x: number }[] = [];

  constructor() {
    this.el.type = "button";
    this.el.className = "welcome-crystal";
    this.el.setAttribute("aria-label", "Spin the Qubero logo");
    this.name();
    this.svg.setAttribute("viewBox", "-12 -8 504 838");
    this.svg.setAttribute("aria-hidden", "true");
    this.svg.setAttribute("fill", "var(--bg)");
    this.svg.setAttribute("stroke", "currentColor");
    this.svg.setAttribute("stroke-width", "12");
    this.svg.setAttribute("stroke-linejoin", "round");
    this.svg.setAttribute("stroke-linecap", "round");
    this.outline.setAttribute("fill", "none");
    this.outline.setAttribute("stroke-width", "19");
    this.svg.append(...this.paths, this.outline);
    this.el.append(this.svg);
    this.draw(0);
    this.el.addEventListener("click", () => {
      // A press that turned the mark has already had its answer, and the
      // spring is busy giving it. Forgotten either way: the next press might
      // be Enter on the keyboard, which has no pointer to say it went nowhere,
      // and a mark still remembering a drag would swallow that too.
      const dragged = this.wandered > A_CLICK;
      this.wandered = 0;
      if (dragged) return;
      this.spin();
    });
    this.el.addEventListener("pointerdown", e => this.grab(e));
    this.el.addEventListener("pointermove", e => this.turn(e));
    this.el.addEventListener("pointerup", e => this.release(e));
    this.el.addEventListener("pointercancel", e => this.release(e));
    this.reduced.addEventListener("change", () => this.name());
  }

  // A mark that will not move is not a control. Under reduced motion the
  // button does nothing worth reaching, so it leaves the tab order and the
  // accessibility tree rather than promising a spin it will not give; the
  // heading beside it is what names the app. The tooltip goes with it, for
  // the same reason: it offers a turn that is not on offer.
  private name(): void {
    const still = this.reduced.matches;
    // Spelt out rather than toggled: an empty `aria-hidden` is not hidden, it
    // is the attribute present and saying nothing.
    if (still) {
      this.el.setAttribute("aria-hidden", "true");
      this.el.removeAttribute("title");
    } else {
      this.el.removeAttribute("aria-hidden");
      this.el.title = "Give it a spin, or drag it round";
    }
    this.el.tabIndex = still ? -1 : 0;
  }

  // Take hold of the mark. The width of the element is half a turn, so the
  // face under the pointer stays roughly under it: a grip on the thing itself
  // rather than a rate the pointer nudges.
  private grab(e: PointerEvent): void {
    // The left button and nothing else. A right-press opens a menu, and a
    // right-drag that turned the mark under it would be a surprise.
    if (this.reduced.matches || this.held !== null || e.button !== 0) return;
    const width = this.el.getBoundingClientRect().width;
    if (width === 0) return;
    this.held = e.pointerId;
    this.from = e.clientX;
    this.radiansPerPx = Math.PI / width;
    this.wandered = 0;
    this.speed = 0;
    this.samples = [{ at: e.timeStamp, x: e.clientX }];
    this.el.dataset.spinning = "true";
    // Capture keeps the turn going when the pointer leaves the mark, which it
    // will, since the mark is small and the drag is not. A pointer already
    // gone by the time this runs cannot be captured and throws rather than
    // failing quietly; the drag is still fine without it, just bounded by the
    // element, so the loss is not worth taking the handler down for.
    try {
      this.el.setPointerCapture(e.pointerId);
    } catch {
      // nothing to hold
    }
  }

  private turn(e: PointerEvent): void {
    if (this.held !== e.pointerId) return;
    const moved = e.clientX - this.from;
    const was = this.wandered;
    this.wandered = Math.max(was, Math.abs(moved));
    this.samples.push({ at: e.timeStamp, x: e.clientX });
    while (this.samples.length > 2 && e.timeStamp - this.samples[0]!.at > RECENT) this.samples.shift();
    // Nothing is drawn until the press is a drag rather than a click's wobble,
    // and crossing that line is when the drag takes the mark off whatever was
    // already turning it. A press that never crosses it leaves a spin alone.
    if (this.wandered <= A_CLICK) return;
    if (was <= A_CLICK) {
      cancelAnimationFrame(this.frame);
      this.frame = 0;
    }
    this.angle = moved * this.radiansPerPx;
    this.draw(this.angle);
  }

  // Let go, and hand the spring whatever speed the last few reports had. A
  // flick keeps turning and comes back; a press held still lets go from where
  // it is and springs back from there.
  private release(e: PointerEvent): void {
    if (this.held !== e.pointerId) return;
    this.held = null;
    if (this.el.hasPointerCapture(e.pointerId)) this.el.releasePointerCapture(e.pointerId);
    const samples = this.samples;
    this.samples = [];
    // A press that stayed put is a click, and the click that follows this is
    // what answers it. Springing back from nowhere would leave an animation
    // running for the spin to find and stand down for, so the mark would take
    // a press and do nothing at all.
    if (this.wandered <= A_CLICK) {
      this.speed = 0;
      // Unless a spin was already running, in which case it still is and the
      // mark is somewhere in it. Snapping to rest here would jump it.
      if (this.frame === 0) {
        this.angle = 0;
        this.draw(0);
        delete this.el.dataset.spinning;
      }
      return;
    }
    // Only what happened just before letting go counts. A press dragged round
    // and then held still for a second was let go from a standstill, and the
    // speed it had on the way there is not the speed it has now.
    const recent = samples.filter(s => e.timeStamp - s.at <= RECENT);
    const first = recent[0], last = recent[recent.length - 1];
    const over = first && last ? last.at - first.at : 0;
    this.speed = over > 0 ? ((last!.x - first!.x) / over) * 1000 * this.radiansPerPx : 0;
    this.springBack();
  }

  // The way back to rest: a damped spring, integrated in small fixed steps so
  // that a hard flick and a slow browser do not each get their own physics.
  private springBack(): void {
    let last = performance.now();
    const tick = (now: number): void => {
      if (!this.el.isConnected) { this.dispose(); return; }
      let left = Math.min((now - last) / 1000, LONGEST_STEP);
      last = now;
      while (left > 0) {
        const step = Math.min(left, SUB_STEP);
        this.speed += (-STIFFNESS * this.angle - DAMPING * this.speed) * step;
        this.angle += this.speed * step;
        left -= step;
      }
      // Settled when it is neither anywhere nor going anywhere. Half a degree
      // and half a degree a second are both under what a redraw would show.
      if (Math.abs(this.angle) < 0.008 && Math.abs(this.speed) < 0.008) {
        this.angle = 0;
        this.speed = 0;
        this.frame = 0;
        // A drag that ended in a cancel has no click coming to forget it.
        this.wandered = 0;
        this.draw(0);
        delete this.el.dataset.spinning;
        // A file that started opening mid-drag gets its turning mark back.
        if (this.busy) this.spin();
        return;
      }
      this.draw(this.angle);
      this.frame = requestAnimationFrame(tick);
    };
    this.frame = requestAnimationFrame(tick);
  }

  private draw(angle: number): void {
    const c = Math.cos(angle), s = Math.sin(angle);
    const rotated = points.map(([x, y, z]) =>
      [240 + (x - 240) * c + z * s, y, -(x - 240) * s + z * c]);
    const faces = facets.map(face => {
      const vertices = face.map(i => rotated[i]!);
      const [a, b, d] = vertices as [number[], number[], number[], ...number[][]];
      const ux = b[0]! - a[0]!, uy = b[1]! - a[1]!, uz = b[2]! - a[2]!;
      const vx = d[0]! - a[0]!, vy = d[1]! - a[1]!, vz = d[2]! - a[2]!;
      const normal = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
      const length = Math.hypot(...normal);
      return { vertices, normal: normal.map(n => n / length), depth: vertices.reduce((sum, v) => sum + v[2]!, 0) / vertices.length };
    }).filter(face => face.normal[2]! > 0.0001).sort((a, b) => a.depth - b.depth);
    this.paths.forEach(path => {
      path.setAttribute("display", "none");
      path.removeAttribute("d");
      path.removeAttribute("fill");
    });
    faces.forEach((face, i) => {
      const path = this.paths[i]!;
      path.removeAttribute("display");
      // A restrained, theme-aware tint lets broad planes read as surfaces.
      const light = -0.4 * face.normal[0]! - 0.5 * face.normal[1]! + 0.75 * face.normal[2]!;
      const ink = 2 + 8 * (1 - Math.max(0, light));
      path.setAttribute("fill", `color-mix(in srgb, var(--bg), currentColor ${ink.toFixed(2)}%)`);
      path.setAttribute("d", face.vertices.map((v, j) => `${j === 0 ? "M" : "L"}${v[0]!.toFixed(2)},${v[1]}`).join(" ") + " Z");
    });
    // The outline is only the current silhouette, never an edge in the mesh.
    const projected = [...rotated].sort((a, b) => a[0]! - b[0]! || a[1]! - b[1]!);
    const cross = (a: number[], b: number[], c: number[]): number =>
      (b[0]! - a[0]!) * (c[1]! - a[1]!) - (b[1]! - a[1]!) * (c[0]! - a[0]!);
    const half = (vertices: number[][]): number[][] => {
      const hull: number[][] = [];
      for (const v of vertices) {
        while (hull.length > 1 && cross(hull[hull.length - 2]!, hull[hull.length - 1]!, v) <= 0) hull.pop();
        hull.push(v);
      }
      hull.pop();
      return hull;
    };
    const hull = [...half(projected), ...half([...projected].reverse())];
    this.outline.setAttribute("d", hull.map((v, i) => `${i === 0 ? "M" : "L"}${v[0]!.toFixed(2)},${v[1]}`).join(" ") + " Z");
  }

  spin(): void {
    if (this.reduced.matches) {
      this.el.animate([{ opacity: 0.5 }, { opacity: 1 }], { duration: 180 });
      return;
    }
    // A hand on the mark, or a spring carrying it home, is already in charge.
    if (this.frame !== 0 || this.held !== null) return;
    this.began = performance.now();
    this.turns = 1;
    this.el.dataset.spinning = "true";
    const tick = (now: number): void => {
      if (!this.el.isConnected) { this.dispose(); return; }
      const elapsed = (now - this.began) / TURN;
      if (this.busy) this.turns = Math.floor(elapsed) + 1;
      if (!this.busy && elapsed >= this.turns) {
        this.draw(0);
        this.frame = 0;
        delete this.el.dataset.spinning;
        return;
      }
      // Each complete revolution starts and ends gently at the original mark.
      const t = elapsed % 1;
      this.draw((t - Math.sin(t * Math.PI * 2) / (Math.PI * 2)) * Math.PI * 2);
      this.frame = requestAnimationFrame(tick);
    };
    this.frame = requestAnimationFrame(tick);
  }

  setBusy(busy: boolean): void {
    this.busy = busy;
    // The name stays put. A label swapped on an element nobody is focused on
    // is announced to nobody, and the status line beside the mark already
    // says which file is opening. The turning is the sighted cue.
    if (busy) this.spin();
  }

  dispose(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.busy = false;
    this.held = null;
    this.angle = 0;
    this.speed = 0;
    this.samples = [];
    delete this.el.dataset.spinning;
  }
}
