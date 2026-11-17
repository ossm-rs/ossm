// The GT2 belt, built live from its route rather than baked into the model, so
// it can follow the parts it runs between. Its centreline lies in one plane: the
// straight runs between anchors and wrap circles (the pulley, the idler stacks)
// are tangent to the circles, with arcs where the belt wraps them. That line is
// extruded across the belt's width. Mirrors tools/cad derived.belt_path.

import {
  Box3,
  BufferAttribute,
  BufferGeometry,
  Group,
  Matrix4,
  Mesh,
  Sphere,
  Vector2,
  Vector3,
} from "three";
import type { Material, Object3D } from "three";
import data from "./ossm-alt.parts.json";

type Vec2 = [number, number];
type Anchor = { anchor: string; at: Vec2 };
type Wrap = { wrap: string[]; centre: Vec2; radius: number; ccw: boolean };
type BeltSpec = {
  node: string;
  plane: { origin: number[]; x: number[]; y: number[]; z: number[] };
  width: number;
  thickness: number;
  /** The belt's real length: what the route doesn't use goes to the tips. */
  length: number;
  route: Array<Anchor | Wrap>;
  /** Route elements in each end's lead through its clamp (0: a bare end). */
  leads?: { front?: number; rear?: number };
};

/** How the belt lies. Routed: all along its route. Hang: only the `end`'s lead is
 *  in place, from its tip to the clamp exit, and the rest hangs off the clamp.
 *  `feed` 0..1 slides the tip in, at a steady speed: from `approach` metres back
 *  along the hanging belt (0), up into the clamp and through to where it ends up (1). */
export type BeltPose =
  | { mode: "routed" }
  | { mode: "hang"; end: "front"; feed: number; approach?: number }
  /** Front end in its clamp; the free tip is carried to the first idler stack,
   *  then fed over the idlers and under the pulley, pulling the sag under the
   *  rail straight. `progress` 0..1. */
  | { mode: "thread"; progress: number }
  /** Everything up to the far idler stack in place; the tip `feed`s (0..1) into
   *  the rear clamp, leaving `slack` metres of extra belt sagging between the
   *  idlers and the rear clamp (0: taut, as routed). */
  | { mode: "rear"; feed: number; slack: number };

// The export writes it (see tools/cad webexport.belt_data); JSON types its pairs as number[].
export const BELT = (data as unknown as { belt?: BeltSpec }).belt;

/** A route element, solved: a bare point or a circle the belt wraps. */
type Circle = { centre: Vector2; radius: number; ccw: boolean };

// --- the centreline, in the belt plane ---------------------------------------

/** The straight run leaving `a` and landing on `b` (radius 0: a point). */
function tangent(a: Circle, b: Circle): [Vector2, Vector2] {
  // Signed radii: the belt sits on the left of a ccw wrap, right of a cw one.
  const ra = a.ccw ? a.radius : -a.radius;
  const rb = b.ccw ? b.radius : -b.radius;
  const v = b.centre.clone().sub(a.centre);
  const dist = v.length();
  const u = v.divideScalar(dist);
  const perp = new Vector2(-u.y, u.x);
  const k = (ra - rb) / dist;
  const h = Math.sqrt(Math.max(0, 1 - k * k));
  const n = u.clone().multiplyScalar(-k).addScaledVector(perp, h);
  return [
    a.centre.clone().addScaledVector(n, -ra),
    b.centre.clone().addScaledVector(n, -rb),
  ];
}

const ARC_STEP = (6 * Math.PI) / 180;

function arc(c: Circle, from: Vector2, to: Vector2): Vector2[] {
  const a0 = Math.atan2(from.y - c.centre.y, from.x - c.centre.x);
  const a1 = Math.atan2(to.y - c.centre.y, to.x - c.centre.x);
  const turn = 2 * Math.PI;
  const mod = (x: number) => ((x % turn) + turn) % turn;
  const sweep = c.ccw ? mod(a1 - a0) : -mod(a0 - a1);
  const n = Math.max(2, Math.floor(Math.abs(sweep) / ARC_STEP));
  const out: Vector2[] = [];
  for (let i = 0; i <= n; i++) {
    const a = a0 + (sweep * i) / n;
    out.push(
      new Vector2(
        c.centre.x + c.radius * Math.cos(a),
        c.centre.y + c.radius * Math.sin(a),
      ),
    );
  }
  return out;
}

/** Lengthens the two tips of `path` equally (straight on) to make it `length`. */
export function withTails(path: Vector2[], length: number): Vector2[] {
  let used = 0;
  for (let i = 1; i < path.length; i++) used += path[i].distanceTo(path[i - 1]);
  const spare = length - used;
  if (spare <= 0 || path.length < 2) return path;
  const out = path.map((p) => p.clone());
  const extend = (tip: number, step: number) => {
    // The run into the tip: from the nearest point that isn't the tip itself.
    let prev = tip + step;
    while (prev >= 0 && prev < out.length && out[prev].distanceTo(out[tip]) < 1e-12) prev += step;
    const d = out[tip].clone().sub(out[prev]).normalize();
    out[tip].addScaledVector(d, spare / 2);
  };
  extend(0, 1);
  extend(out.length - 1, -1);
  return out;
}

/** Centreline through [start, circles…, end]. */
export function beltPath(points: Circle[]): Vector2[] {
  return routePath(points).path;
}

/** The centreline, and for each route element the index of the path point
 *  where the belt leaves it. */
function routePath(points: Circle[]): { path: Vector2[]; leaves: number[] } {
  const out: Vector2[] = [points[0].centre.clone()];
  const leaves: number[] = [];
  let arrive: Vector2 | null = null;
  for (let i = 0; i + 1 < points.length; i++) {
    const a = points[i];
    const [leave, land] = tangent(a, points[i + 1]);
    if (a.radius > 0 && arrive) out.push(...arc(a, arrive, leave).slice(1));
    leaves.push(out.length);
    out.push(leave, land);
    arrive = land;
  }
  leaves.push(out.length - 1);
  return { path: out, leaves };
}

// --- slack: the free belt's shape, without physics ---------------------------

// How a free length hangs: carried straight on for a stub (the belt is stiff),
// then bent round towards straight down, then hanging.
const HANG_STUB = 0.012;
const HANG_BEND = 0.03;
const DOWN = new Vector2(0, -1); // the belt plane's y is up

/** `length` of belt hanging from `from`, leaving in direction `dir`. */
function hang(from: Vector2, dir: Vector2, length: number): Vector2[] {
  const out = [from.clone()];
  let left = length;
  const d = dir.clone().normalize();
  const stub = Math.min(HANG_STUB, left);
  const p = from.clone().addScaledVector(d, stub);
  out.push(p.clone());
  left -= stub;
  // Turn towards down, the short way round.
  const turn = Math.atan2(d.x * DOWN.y - d.y * DOWN.x, d.dot(DOWN));
  const sweep = Math.min(Math.abs(turn), left / HANG_BEND) * Math.sign(turn);
  if (Math.abs(sweep) > 1e-6) {
    const ccw = sweep > 0;
    const normal = ccw ? new Vector2(-d.y, d.x) : new Vector2(d.y, -d.x);
    const centre = p.clone().addScaledVector(normal, HANG_BEND);
    const a0 = Math.atan2(p.y - centre.y, p.x - centre.x);
    const n = Math.max(2, Math.ceil(Math.abs(sweep) / ARC_STEP));
    for (let i = 1; i <= n; i++) {
      const a = a0 + (sweep * i) / n;
      out.push(new Vector2(centre.x + HANG_BEND * Math.cos(a), centre.y + HANG_BEND * Math.sin(a)));
    }
    left -= Math.abs(sweep) * HANG_BEND;
  }
  if (left > 1e-6) {
    // On in the direction the bend ends in (not its last facet's).
    const end = d.clone().rotateAround(new Vector2(), sweep);
    out.push(out[out.length - 1].clone().addScaledVector(end, left));
  }
  return out;
}

function cubic(p0: Vector2, p1: Vector2, p2: Vector2, p3: Vector2, n: number): Vector2[] {
  const out: Vector2[] = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const u = 1 - t;
    out.push(
      p0
        .clone()
        .multiplyScalar(u * u * u)
        .addScaledVector(p1, 3 * u * u * t)
        .addScaledVector(p2, 3 * u * t * t)
        .addScaledVector(p3, t * t * t),
    );
  }
  return out;
}

/** `length` of stiff belt sagging from `a` (leaving along `da`) to `b`
 *  (arriving along `db`): straight for `stubA`/`stubB` at each end, then a
 *  smooth dip, its depth found to take up the length. From `a` to `b`. */
function droop(
  a: Vector2,
  da: Vector2,
  b: Vector2,
  db: Vector2,
  length: number,
  stubA: number,
  stubB: number,
): Vector2[] {
  const sa = Math.min(stubA, length / 4);
  const sb = Math.min(stubB, length / 4);
  const a1 = a.clone().addScaledVector(da, sa);
  const b1 = b.clone().addScaledVector(db, -sb);
  const middle = length - sa - sb;
  const chord = a1.distanceTo(b1);
  if (middle <= chord * (1 + 1e-6)) return [a.clone(), a1, b1, b.clone()];
  const e = b1.clone().sub(a1).normalize();
  const half = chord / 2;
  const mid = a1.clone().lerp(b1, 0.5);
  // Two curves meeting at the low point, level there, each leaving its end
  // along the belt's direction.
  const shape = (depth: number) => {
    const m = mid.clone().addScaledVector(DOWN, depth);
    // Out from each end, but never past the middle (a deep loop would cross itself).
    const reach = Math.min(0.45 * (half + depth), 0.9 * half);
    const lean = 0.55 * half;
    const n = 24;
    return [
      ...cubic(a1, a1.clone().addScaledVector(da, reach), m.clone().addScaledVector(e, -lean), m, n),
      ...cubic(m, m.clone().addScaledVector(e, lean), b1.clone().addScaledVector(db, -reach), b1, n).slice(1),
    ];
  };
  const measure = (pts: Vector2[]) => {
    let l = 0;
    for (let i = 1; i < pts.length; i++) l += pts[i].distanceTo(pts[i - 1]);
    return l;
  };
  // The curve's length grows with its depth: bisect for the one that fits.
  let lo = 0;
  let hi = middle;
  for (let i = 0; i < 40; i++) {
    const d = (lo + hi) / 2;
    if (measure(shape(d)) < middle) lo = d;
    else hi = d;
  }
  return [a.clone(), ...shape((lo + hi) / 2), b.clone()];
}

/** `length` of belt hanging from `from` (leaving along `dir`, as `hang`) and
 *  held up by its tip at `tip`: down from the clamp, a U at the bottom, and
 *  straight up to the tip. With the tip at the bottom it is just `hang`. */
function held2u(from: Vector2, dir: Vector2, tip: Vector2, length: number): Vector2[] {
  // The clamp side: its stub and bend, then straight down.
  const side = hang(from, dir, length + 10);
  const bendEnd = side[side.length - 2];
  const sideCum = lengths(side);
  const bendLen = sideCum[side.length - 2];
  const r = Math.abs(tip.x - bendEnd.x) / 2;
  // Where the U bottoms out, so the whole thing is `length` long.
  const bottom = (bendEnd.y + tip.y + bendLen + Math.PI * r - length) / 2;
  if (bottom > Math.min(bendEnd.y, tip.y)) {
    // Not enough belt to hang down at all: straight from the bend to the tip.
    return [...cut(side, sideCum, 0, bendLen), tip.clone()];
  }
  const out = cut(side, sideCum, 0, bendLen);
  out.push(new Vector2(bendEnd.x, bottom));
  if (r > 1e-6) {
    const cx = (bendEnd.x + tip.x) / 2;
    const a0 = bendEnd.x < tip.x ? Math.PI : 0;
    const sweep = bendEnd.x < tip.x ? Math.PI : -Math.PI; // round the bottom
    const n = 12;
    for (let i = 1; i <= n; i++) {
      const a = a0 + (sweep * i) / n;
      out.push(new Vector2(cx + r * Math.cos(a), bottom + r * Math.sin(a)));
    }
  }
  out.push(tip.clone());
  return out;
}

/** `n` points evenly spaced along `path`. */
function resample(path: Vector2[], n: number): Vector2[] {
  const cum = lengths(path);
  const total = cum[cum.length - 1];
  return Array.from({ length: n }, (_, i) => at(path, cum, (total * i) / (n - 1)));
}

/** From curve `a` to curve `b` (both from the same start, the same length):
 *  blends the direction of each stretch along them, so the belt keeps its length. */
function blend(a: Vector2[], b: Vector2[], w: number): Vector2[] {
  const n = 120;
  const ra = resample(a, n);
  const rb = resample(b, n);
  const out = [ra[0].clone()];
  for (let i = 1; i < n; i++) {
    const da = ra[i].clone().sub(ra[i - 1]);
    const db = rb[i].clone().sub(rb[i - 1]);
    const aa = Math.atan2(da.y, da.x);
    let turn = Math.atan2(db.y, db.x) - aa;
    turn = Math.atan2(Math.sin(turn), Math.cos(turn)); // the short way round
    const angle = aa + turn * w;
    const len = da.length() + (db.length() - da.length()) * w;
    out.push(out[i - 1].clone().add(new Vector2(Math.cos(angle), Math.sin(angle)).multiplyScalar(len)));
  }
  return out;
}

/** Arc length at each point of `path`. */
function lengths(path: Vector2[]): number[] {
  const out = [0];
  for (let i = 1; i < path.length; i++) out.push(out[i - 1] + path[i].distanceTo(path[i - 1]));
  return out;
}

/** The point at arc length `s` along `path` (straight on past either end). */
function at(path: Vector2[], cum: number[], s: number): Vector2 {
  if (s <= 0) {
    const d = path[0].clone().sub(firstApart(path, 0, 1)).normalize();
    return path[0].clone().addScaledVector(d, -s);
  }
  const last = path.length - 1;
  if (s >= cum[last]) {
    const d = path[last].clone().sub(firstApart(path, last, -1)).normalize();
    return path[last].clone().addScaledVector(d, s - cum[last]);
  }
  let i = 1;
  while (i < last && cum[i] < s) i++;
  const t = (s - cum[i - 1]) / Math.max(cum[i] - cum[i - 1], 1e-12);
  return path[i - 1].clone().lerp(path[i], Math.min(t, 1));
}

/** The nearest point after `i` (stepping by `step`) that isn't on top of it. */
function firstApart(path: Vector2[], i: number, step: number): Vector2 {
  let j = i + step;
  while (j >= 0 && j < path.length && path[j].distanceTo(path[i]) < 1e-12) j += step;
  return path[Math.min(Math.max(j, 0), path.length - 1)];
}

/** `path` between arc lengths s0 < s1 (s0 may be before its start). */
function cut(path: Vector2[], cum: number[], s0: number, s1: number): Vector2[] {
  const out = [at(path, cum, s0)];
  for (let i = 0; i < path.length; i++) if (cum[i] > s0 && cum[i] < s1) out.push(path[i].clone());
  out.push(at(path, cum, s1));
  return out;
}

// --- the mesh ----------------------------------------------------------------

// Path points the geometry has room for (arcs are a few dozen each).
const MAX_POINTS = 512;
// Per segment: 4 sides × 2 triangles; plus 2 end caps × 2 triangles.
const MAX_VERTS = ((MAX_POINTS - 1) * 8 + 4) * 3;

function emptyGeometry(): BufferGeometry {
  const g = new BufferGeometry();
  g.setAttribute("position", new BufferAttribute(new Float32Array(MAX_VERTS * 3), 3));
  g.setAttribute("normal", new BufferAttribute(new Float32Array(MAX_VERTS * 3), 3));
  return g;
}

/** Writes the belt along `pts` (3D, in its parent's frame) into `g`. */
function extrude(
  g: BufferGeometry,
  pts: Vector3[],
  across: Vector3,
  width: number,
  thickness: number,
) {
  const pos = g.getAttribute("position") as BufferAttribute;
  const nrm = g.getAttribute("normal") as BufferAttribute;
  const n = Math.min(pts.length, MAX_POINTS);
  // In-plane normal at each point (perpendicular to the run).
  const side: Vector3[] = [];
  for (let i = 0; i < n; i++) {
    const t = pts[Math.min(i + 1, n - 1)].clone().sub(pts[Math.max(i - 1, 0)]).normalize();
    side.push(new Vector3().crossVectors(across, t).normalize());
  }
  const corner = (i: number, s: number, z: number) =>
    pts[i]
      .clone()
      .addScaledVector(side[i], (s * thickness) / 2)
      .addScaledVector(across, (z * width) / 2);
  let v = 0;
  const tri = (a: Vector3, b: Vector3, c: Vector3, out: Vector3[]) => {
    // Wind each triangle to face its outward normal.
    const face = new Vector3().subVectors(b, a).cross(new Vector3().subVectors(c, a));
    const flip = face.dot(out[0]) < 0;
    const order = flip ? [a, c, b] : [a, b, c];
    const outs = flip ? [out[0], out[2], out[1]] : out;
    for (let k = 0; k < 3; k++) {
      pos.setXYZ(v, order[k].x, order[k].y, order[k].z);
      nrm.setXYZ(v, outs[k].x, outs[k].y, outs[k].z);
      v++;
    }
  };
  // The four long faces: the two belt faces (±side) and the two edges (±across).
  const faces: Array<[number, number, number, number]> = [
    [1, -1, 1, 1], // +side
    [-1, -1, -1, 1], // -side
    [-1, 1, 1, 1], // +across
    [-1, -1, 1, -1], // -across
  ];
  for (let i = 0; i + 1 < n; i++) {
    for (const [s0, z0, s1, z1] of faces) {
      const p = [corner(i, s0, z0), corner(i, s1, z1), corner(i + 1, s1, z1), corner(i + 1, s0, z0)];
      const out = (j: number) =>
        s0 === s1 ? side[j].clone().multiplyScalar(s0) : across.clone().multiplyScalar(z0);
      const o = [out(i), out(i), out(i + 1), out(i + 1)];
      tri(p[0], p[1], p[2], [o[0], o[1], o[2]]);
      tri(p[0], p[2], p[3], [o[0], o[2], o[3]]);
    }
  }
  // End caps.
  for (const [i, j] of [
    [0, 1],
    [n - 1, n - 2],
  ]) {
    const out = pts[i].clone().sub(pts[j]).normalize();
    const p = [corner(i, -1, -1), corner(i, 1, -1), corner(i, 1, 1), corner(i, -1, 1)];
    tri(p[0], p[1], p[2], [out, out, out]);
    tri(p[0], p[2], p[3], [out, out, out]);
  }
  pos.needsUpdate = true;
  nrm.needsUpdate = true;
  g.setDrawRange(0, v);
  // Framing and culling read these; only the vertices in use count (the rest of
  // the buffer is spare room).
  const box = (g.boundingBox ??= new Box3()).makeEmpty();
  const p = new Vector3();
  for (let k = 0; k < v; k++) box.expandByPoint(p.fromBufferAttribute(pos, k));
  box.getBoundingSphere((g.boundingSphere ??= new Sphere()));
}

// --- following the parts -------------------------------------------------------

/** A route point carried by a node: where it sits in that node's own frame. */
type Carried = { node: Object3D; local: Vector3 };

/** The belt in a copy of the model: `update()` re-solves it from where the
 *  nodes it runs between are now. */
export class Belt {
  readonly mesh: Mesh;
  private readonly spec: BeltSpec;
  private readonly origin: Vector3;
  private readonly x: Vector3;
  private readonly y: Vector3;
  private readonly z: Vector3;
  private readonly carried: Array<{ el: Anchor | Wrap; at: Carried | null }>;
  private pose: BeltPose = { mode: "routed" };
  /** Each tip's length past its clamp when routed with every part at home. */
  private tail = 0;
  /** The rear lead's length (clamp entry to tip point) with the parts at home. */
  private rearLead = 0;

  constructor(spec: BeltSpec, model: Object3D, material: Material) {
    this.spec = spec;
    this.origin = new Vector3(...spec.plane.origin);
    this.x = new Vector3(...spec.plane.x);
    this.y = new Vector3(...spec.plane.y);
    this.z = new Vector3(...spec.plane.z);
    this.mesh = new Mesh(emptyGeometry(), material);
    this.mesh.name = spec.node;
    model.updateMatrixWorld(true);
    const inverse = (o: Object3D) => new Matrix4().copy(o.matrixWorld).invert();
    // Each anchor or wrap rides on its (first) node, from where it is now.
    this.carried = spec.route.map((el) => {
      const name = "anchor" in el ? el.anchor : el.wrap[0];
      const node = model.getObjectByName(name);
      if (!node) {
        console.warn(`[belt] node not found: ${name}`);
        return { el, at: null };
      }
      const p = "anchor" in el ? el.at : el.centre;
      return { el, at: { node, local: this.to3(p).applyMatrix4(inverse(node)) } };
    });
    // Measured with everything in its assembled place, as now.
    const { path, leaves } = routePath(this.circles());
    const cum = lengths(path);
    const routeLength = cum[cum.length - 1];
    this.tail = Math.max(spec.length - routeLength, 0) / 2;
    const marks = this.marks(cum, leaves);
    this.rearLead = marks ? routeLength - marks.rearEntry : 0;
  }

  /** The route elements where they are now, in the belt plane. */
  private circles(): Circle[] {
    return this.carried.map(({ el, at }) => {
      const world = at
        ? at.local.clone().applyMatrix4(at.node.matrixWorld)
        : this.to3("anchor" in el ? el.at : el.centre);
      const centre = this.to2(world);
      return "anchor" in el
        ? { centre, radius: 0, ccw: true }
        : { centre, radius: el.radius, ccw: el.ccw };
    });
  }

  /** Arc lengths along the route where the belt leaves the front clamp, lands
   *  on the near idler stack, leaves the far one and enters the rear clamp.
   *  The route is [front lead…, idlers, pulley, idlers, rear lead…]. */
  private marks(cum: number[], leaves: number[]) {
    const front = this.spec.leads?.front ?? 0;
    const rear = this.spec.leads?.rear ?? 0;
    if (!front || !rear) return null;
    const far = front + 2; // the far idler stack's route element
    return {
      exit: cum[leaves[front - 1]],
      land: cum[leaves[front - 1] + 1],
      leave: cum[leaves[far]],
      rearEntry: cum[leaves[far] + 1],
    };
  }

  private centreline(circles: Circle[]): Vector2[] {
    const { path, leaves } = routePath(circles);
    const total = this.spec.length;
    const pose = this.pose;
    if (pose.mode === "routed") return withTails(path, total);
    const cum = lengths(path);
    const pt = (s: number) => at(path, cum, s);
    // Directions along the straight run leaving (after) or arriving at (before)
    // a point: one-sided, so a neighbouring arc's facets don't tilt them.
    const after = (s: number) => pt(s + 1e-4).sub(pt(s)).normalize();
    const before = (s: number) => pt(s).sub(pt(s - 1e-4)).normalize();
    const front = this.spec.leads?.front ?? 0;
    const exit = front > 0 ? cum[leaves[front - 1]] : 0;
    // The front tip's final place (route arc length; negative: past the outlet).
    const frontTip = -this.tail;

    if (pose.mode === "hang") {
      // The front lead is in place from the tip to where it leaves the clamp;
      // the rest hangs from there. The tip travels from `approach` past the exit
      // (out on the hanging belt) to its final place in the clamp.
      const approach = pose.approach ?? 0;
      const feed = Math.min(Math.max(pose.feed, 0), 1);
      const tip = exit + approach + (frontTip - exit - approach) * feed;
      if (tip >= exit) {
        // Not in the clamp yet: the tip is `tip - exit` along the hanging curve
        // (which stays put), and the belt hangs on from there.
        const ahead = tip - exit;
        const curve = hang(pt(exit), after(exit), ahead + total);
        return cut(curve, lengths(curve), ahead, ahead + total);
      }
      const held = cut(path, cum, tip, exit);
      const free = hang(held[held.length - 1], after(exit), Math.max(total - (exit - tip), 0));
      return [...held, ...free.slice(1)];
    }

    const m = this.marks(cum, leaves);
    if (!m) return withTails(path, total);
    // Belt past the far idler stack once everything up to it is in place.
    const beyond = total - (m.leave - frontTip);

    if (pose.mode === "thread") {
      const held = cut(path, cum, frontTip, exit);
      const free = total - (exit - frontTip); // belt past the front clamp
      const exitAt = pt(exit);
      const hanging = hang(exitAt, after(exit), free);
      const hangTip = hanging[hanging.length - 1];
      // The tip's way: lifted from where it hangs to just short of the near
      // idlers, along the run onto them, then over and under the route and out
      // past the far idlers.
      const landAt = pt(m.land);
      const run = before(m.land);
      const lead = 0.06;
      const short = landAt.clone().addScaledVector(run, -lead);
      // The sag ends this far short of the idlers and the belt runs straight
      // onto them from there, so it doesn't come up through them from below.
      const pin = 0.03; // just outside the housing wall (it starts ~27 mm before the idlers)
      // Straight out of the front clamp and onto the pin before the sag dips.
      const stubOut = 0.015;
      const stubIn = 0.006;
      const exitDir = after(exit);
      const pinAt = landAt.clone().addScaledVector(run, -pin);
      const lift = hangTip.distanceTo(short);
      const through = m.leave - m.land + beyond;
      const u = Math.min(Math.max(pose.progress, 0), 1) * (lift + lead + through);
      if (u <= lift + lead) {
        const tip =
          u <= lift
            ? hangTip.clone().lerp(short, u / Math.max(lift, 1e-9))
            : short.clone().addScaledVector(run, u - lift);
        // Past the pin, the end is held straight along the run.
        const along = u - lift - (lead - pin);
        if (along > 0) {
          const span = droop(exitAt, exitDir, pinAt, run, free - along, stubOut, stubIn);
          return [...held, ...span.slice(1), tip];
        }
        // Lifting: held by the tip, the belt hangs in a U from the clamp to the hand.
        const loop = held2u(exitAt, exitDir, tip, free);
        if (u <= lift) return [...held, ...loop.slice(1)];
        // Along the run to the pin: settling from the U into the sag.
        const w = (u - lift) / Math.max(lead - pin, 1e-9);
        const span = droop(exitAt, exitDir, tip, run, free, stubOut, 0);
        return [...held, ...blend(loop, span, w * w * (3 - 2 * w)).slice(1)];
      }
      // Fed past the near idlers by `fed`: the sag under the rail takes up the rest.
      const fed = u - lift - lead;
      const mech = m.leave - m.land;
      const ahead =
        fed <= mech
          ? cut(path, cum, m.land, m.land + fed)
          : [...cut(path, cum, m.land, m.leave), pt(m.leave).addScaledVector(after(m.leave), fed - mech)];
      const span = droop(exitAt, exitDir, pinAt, run, free - fed - pin, stubOut, stubIn);
      return [...held, ...span.slice(1), ...ahead];
    }

    // mode "rear": in place up to the far idlers; the rest is `beyond` long.
    const held = cut(path, cum, frontTip, m.leave);
    const leaveAt = pt(m.leave);
    const gap = m.rearEntry - m.leave; // straight, idlers to the rear clamp
    if (beyond < gap) {
      // Can't reach the clamp yet: held out straight towards it.
      return [...held, leaveAt.clone().addScaledVector(after(m.leave), beyond)];
    }
    // How much is fed into the rear clamp: up to its final depth less the slack,
    // and never so much that the span to the clamp would have to stretch.
    const depth = this.rearLead + this.tail - Math.max(pose.slack, 0);
    const inClamp = Math.max(Math.min(Math.min(Math.max(pose.feed, 0), 1) * depth, beyond - gap), 0);
    // As at the front of step 18, mirrored: held straight off the idlers until
    // clear of the housing wall, then a smooth dip, straight into the clamp's slot.
    const pinOut = Math.min(0.03, gap);
    const leaveDir = after(m.leave);
    const pinAt = leaveAt.clone().addScaledVector(leaveDir, pinOut);
    const span = [
      leaveAt,
      ...droop(pinAt, leaveDir, pt(m.rearEntry), before(m.rearEntry), beyond - inClamp - pinOut, 0.006, 0.015),
    ];
    const into = cut(path, cum, m.rearEntry, m.rearEntry + inClamp);
    return [...held, ...span.slice(1), ...(inClamp > 0 ? into.slice(1) : [])];
  }

  private to3([u, v]: Vec2 | number[]): Vector3 {
    return this.origin.clone().addScaledVector(this.x, u).addScaledVector(this.y, v);
  }

  private to2(p: Vector3): Vector2 {
    const q = p.clone().sub(this.origin);
    return new Vector2(q.dot(this.x), q.dot(this.y));
  }

  /** How the belt should lie from the next `update()` on. */
  setPose(pose: BeltPose) {
    this.pose = pose;
  }

  /** Re-solve the belt from its nodes' current places (call after moving them). */
  update() {
    const parent = this.mesh.parent;
    parent?.updateWorldMatrix(true, false);
    const toParent = parent ? new Matrix4().copy(parent.matrixWorld).invert() : new Matrix4();
    const pts = this.centreline(this.circles()).map((p) =>
      this.to3([p.x, p.y]).applyMatrix4(toParent),
    );
    extrude(this.mesh.geometry, pts, this.z, this.spec.width, this.spec.thickness);
  }
}

/** Adds the belt to a copy of the model (under a "belt" group), solved in place. */
export function addBelt(model: Object3D, material: Material): Belt | null {
  if (!BELT) return null;
  const belt = new Belt(BELT, model, material);
  let group = model.getObjectByName("belt");
  if (!group) {
    group = new Group();
    group.name = "belt";
    model.add(group);
  }
  group.add(belt.mesh);
  belt.update();
  return belt;
}
