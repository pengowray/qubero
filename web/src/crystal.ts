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

  constructor() {
    this.el.type = "button";
    this.el.className = "welcome-crystal";
    this.el.setAttribute("aria-label", "Spin the Qubero crystal");
    this.el.title = "Give it a spin";
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
    this.el.addEventListener("click", () => this.spin());
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
    if (this.frame !== 0) return;
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
    this.el.setAttribute("aria-label", busy ? "Opening file" : "Spin the Qubero crystal");
    if (busy) this.spin();
  }

  dispose(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.busy = false;
    delete this.el.dataset.spinning;
  }
}
