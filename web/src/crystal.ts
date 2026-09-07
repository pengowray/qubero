// The opening-screen mark, rebuilt from the old drawing. Its rest projection
// keeps the broad bevels and unequal long faces instead of replacing the ends
// with a regular pyramid. A shallow solid gives those drawn facets real depth
// when the mark turns; the back shares the same deliberately irregular cut.
type Vertex = readonly [number, number, number];
const points: Vertex[] = [
  [285, 10, 0], [140, 100, 0], [65, 160, 0], [43, 210, 0],
  [12, 480, 0], [20, 540, 0], [59, 575, 0], [163, 760, 0],
  [194, 808, 0], [401, 685, 0], [426, 625, 0], [449, 530, 0],
  [465, 281, 0], [433, 235, 0], [355, 108, 0],
  [100, 158, 40], [105, 237, 70], [170, 272, 120], [308, 294, 155],
  [359, 235, 100], [76, 345, 65], [49, 500, 30], [131, 660, 70],
  [201, 670, 90], [273, 609, 140], [379, 608, 80],
];
const facets = [
  [2, 1, 15], [0, 1, 15, 16, 17, 18, 19, 14], [14, 19, 13],
  [13, 19, 18, 24, 25, 11, 12], [18, 17, 20, 21, 6, 22, 23, 24],
  [17, 16, 20], [2, 15, 16, 20, 21, 4, 3], [4, 21, 6, 5],
  [6, 22, 7], [22, 23, 7], [7, 23, 24, 25, 10, 9, 8], [25, 11, 10],
];
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
  private readonly paths = Array.from({ length: facets.length * 2 }, () => document.createElementNS(NS, "path"));
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
    const faces = [-1, 1].flatMap(side => facets.map(face => {
      const vertices = face.map(i => {
        const [x, y, z] = points[i]!;
        return [240 + (x - 240) * c + side * z * s, y, -(x - 240) * s + side * z * c];
      });
      return { vertices, depth: vertices.reduce((sum, v) => sum + v[2]!, 0) / vertices.length };
    })).sort((a, b) => a.depth - b.depth);
    faces.forEach((face, i) => {
      const path = this.paths[i]!;
      path.setAttribute("d", face.vertices.map((v, j) => `${j === 0 ? "M" : "L"}${v[0]!.toFixed(2)},${v[1]}`).join(" ") + " Z");
    });
    // Give the silhouette the heavier ink of the original. Its convex hull
    // follows the turning solid, including the edges that emerge in profile.
    const projected = faces.flatMap(face => face.vertices).sort((a, b) => a[0]! - b[0]! || a[1]! - b[1]!);
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
