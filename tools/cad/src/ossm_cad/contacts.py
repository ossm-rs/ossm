"""Contact detection and removal-direction analysis for placed parts.

Works on the tessellated surfaces: each unit's surface is sampled densely and
distances are exact point-to-triangle (accurate to the tessellation
deflection), with candidate triangles found via a KD-tree over the samples.

Removal along a unit direction v is tested against each neighbour B:

- points of A touching B (opposing faces within `tol`) must not push into
  it: v·n <= 0 for A's outward normal n there;
- points of A within `band` of B are swept `travel` mm along v and must not
  reach B.

Interpenetration is reported but doesn't block: upstream models screws
overlapping the printed hole by about the thread depth, and the overlap's
normals (chamfered tips, thread flanks) would otherwise forbid unscrewing.

The sweep matters for fasteners: a bolt shank sits in a clearance hole
without touching it, yet the hole still pins it to its axis. (A linearised
gap constraint fails here: next to fillets and hole rims the obstacle curves
away, and it reports collisions that never happen.)
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

import numpy as np
from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.TopoDS import TopoDS_Shape
from scipy.spatial import cKDTree

from .geom import _TYPE_NAMES, bbox, faces, transformed
from .render import Mesh, tessellate
from .step import Assembly, Instance


@dataclass
class Unit:
    """Something that moves as one during assembly: a part, or a rigid bought-in sub-assembly."""

    name: str
    path: tuple[str, ...]
    shape: TopoDS_Shape  # world-placed
    lo: np.ndarray
    hi: np.ndarray
    members: list[Instance] = field(default_factory=list)
    _surf: "Surface | None" = None

    @property
    def surface(self) -> "Surface":
        if self._surf is None:
            self._surf = Surface.of(self.shape)
        return self._surf


def units(asm: Assembly, rigid: list[str]) -> tuple[list[Unit], list[Instance]]:
    """Leaf solids grouped into movable units.

    Sub-assemblies whose name matches a `rigid` regex (bought-in parts like the
    motor or PCB) become one unit. Returns (units, dropped duplicate instances).
    """
    from OCP.BRep import BRep_Builder
    from OCP.TopoDS import TopoDS_Compound

    pats = [re.compile(p, re.I) for p in rigid]
    leaves, dropped = dedupe(asm.leaf_instances())
    groups: dict[tuple[str, ...], list[Instance]] = {}
    for inst in leaves:
        key = inst.path
        for i, seg in enumerate(inst.path[:-1]):
            if any(p.search(seg) for p in pats):
                key = inst.path[: i + 1]
                break
        groups.setdefault(key, []).append(inst)

    out = []
    for path, members in groups.items():
        builder = BRep_Builder()
        comp = TopoDS_Compound()
        builder.MakeCompound(comp)
        for m in members:
            builder.Add(comp, transformed(m.proto.shape, m.world))
        lo, hi = bbox(comp)
        name = members[0].proto.name if len(members) == 1 and path == members[0].path else path[-1]
        out.append(Unit(display_name(name, path), path, comp, lo, hi, members))
    return out, dropped


def dedupe(instances: list[Instance], tol: float = 0.5) -> tuple[list[Instance], list[Instance]]:
    """Drop instances that put an identical solid in (nearly) the same place.

    Upstream exports sometimes contain a sub-assembly twice: inserted and also
    referenced, or mirrored onto itself ("Rail (Mirror)" 0.1 mm from the
    original). Compare geometry, not names: same volume, bounds within `tol` mm.
    """
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps

    vols: dict[str, float] = {}
    keep: list[tuple[Instance, float, np.ndarray, np.ndarray]] = []
    dropped = []
    for inst in instances:
        if inst.proto.entry not in vols:
            g = GProp_GProps()
            BRepGProp.VolumeProperties_s(inst.proto.shape, g)
            vols[inst.proto.entry] = g.Mass()
        v = vols[inst.proto.entry]
        lo, hi = bbox(transformed(inst.proto.shape, inst.world))
        if any(
            abs(v - kv) <= 1e-3 * max(v, kv) and np.all(np.abs(lo - klo) <= tol) and np.all(np.abs(hi - khi) <= tol)
            for _, kv, klo, khi in keep
        ):
            dropped.append(inst)
        else:
            keep.append((inst, v, lo, hi))
    return [k[0] for k in keep], dropped


def display_name(name: str, path: tuple[str, ...]) -> str:
    # Anonymous Fusion component bodies ("=>[0:1:1:43]" -> SOLID) take their parent's name.
    if name in ("SOLID", "COMPOUND", "") and len(path) >= 2:
        parent = re.sub(r":\d+$", "", path[-2])
        return f"{parent} (body)"
    return re.sub(r":\d+$", "", name)


@dataclass
class Surface:
    """Tessellated surface with dense samples, for proximity queries."""

    mesh: Mesh
    face_kind: list[str]  # B-rep surface type per face index
    tri_a: np.ndarray
    tri_b: np.ndarray
    tri_c: np.ndarray
    tri_n: np.ndarray  # outward unit normals
    tree: cKDTree  # over surface samples
    pts: np.ndarray  # surface samples
    pts_n: np.ndarray
    pts_tri: np.ndarray

    @classmethod
    def of(cls, shape: TopoDS_Shape, spacing: float = 0.6) -> "Surface":
        mesh = tessellate(shape, 0.05)
        kinds = [_TYPE_NAMES.get(int(BRepAdaptor_Surface(f).GetType()), "other") for f in faces(shape)]
        v, t = mesh.verts, mesh.tris
        a, b, c = v[t[:, 0]], v[t[:, 1]], v[t[:, 2]]
        n = np.cross(b - a, c - a)
        area2 = np.linalg.norm(n, axis=1)
        n = np.divide(n, area2[:, None], out=np.zeros_like(n), where=area2[:, None] > 0)

        # Uniform area sampling: k points per triangle in proportion to its area.
        k = np.maximum(1, np.ceil(area2 / 2 / spacing**2)).astype(int)
        tri_idx = np.repeat(np.arange(len(t)), k)
        rng = np.random.default_rng(0)
        r1, r2 = rng.random(len(tri_idx)), rng.random(len(tri_idx))
        flip = r1 + r2 > 1
        r1[flip], r2[flip] = 1 - r1[flip], 1 - r2[flip]
        pts = a[tri_idx] + r1[:, None] * (b - a)[tri_idx] + r2[:, None] * (c - a)[tri_idx]
        return cls(mesh, kinds, a, b, c, n, cKDTree(pts), pts, n[tri_idx], tri_idx)

    def distance(self, p: np.ndarray, upper: float, k: int = 8) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
        """Distance from each point to this surface (inf beyond `upper`), closest point, and
        the outward normal of the triangle it lies on.

        The k nearest surface samples nominate candidate triangles; the exact
        point-triangle distance is taken over those.
        """
        dd, ii = self.tree.query(p, k=k, distance_upper_bound=upper + 1.0)
        valid = np.isfinite(dd)
        dist = np.full(len(p), np.inf)
        normal = np.zeros_like(p)
        closest = np.zeros_like(p)
        rows = np.nonzero(valid.any(axis=1))[0]
        if len(rows) == 0:
            return dist, closest, normal
        ii = np.where(valid[rows], ii[rows], ii[rows][:, :1])  # pad misses with the nearest hit
        tris = self.pts_tri[ii]  # (R, k)
        P = np.repeat(p[rows], k, axis=0)
        T = tris.ravel()
        q = _closest_on_tris(P, self.tri_a[T], self.tri_b[T], self.tri_c[T])
        d = np.linalg.norm(q - P, axis=1).reshape(-1, k)
        j = np.argmin(d, axis=1)
        r = np.arange(len(rows))
        dist[rows] = d[r, j]
        qj = q.reshape(-1, k, 3)[r, j]
        normal[rows] = self.tri_n[tris[r, j]]
        closest[rows] = qj
        dist[dist > upper] = np.inf
        return dist, closest, normal


def _closest_on_tris(p, a, b, c):
    """Closest point on triangle (a[i], b[i], c[i]) to p[i]. Ericson, RTCD 5.1.5."""
    ab, ac, ap = b - a, c - a, p - a
    d1, d2 = np.einsum("ij,ij->i", ab, ap), np.einsum("ij,ij->i", ac, ap)
    bp = p - b
    d3, d4 = np.einsum("ij,ij->i", ab, bp), np.einsum("ij,ij->i", ac, bp)
    cp = p - c
    d5, d6 = np.einsum("ij,ij->i", ab, cp), np.einsum("ij,ij->i", ac, cp)
    vc = d1 * d4 - d3 * d2
    vb = d5 * d2 - d1 * d6
    va = d3 * d6 - d5 * d4
    with np.errstate(divide="ignore", invalid="ignore"):
        denom = 1.0 / (va + vb + vc)
        v = vb * denom
        w = vc * denom
        out = a + ab * v[:, None] + ac * w[:, None]
        # Edge and vertex regions.
        m = (d1 <= 0) & (d2 <= 0)
        out[m] = a[m]
        m2 = (d3 >= 0) & (d4 <= d3)
        out[m2 & ~m] = b[m2 & ~m]
        done = m | m2
        m3 = (vc <= 0) & (d1 >= 0) & (d3 <= 0) & ~done
        t = d1 / (d1 - d3)
        out[m3] = (a + ab * t[:, None])[m3]
        done |= m3
        m4 = (d6 >= 0) & (d5 <= d6) & ~done
        out[m4] = c[m4]
        done |= m4
        m5 = (vb <= 0) & (d2 >= 0) & (d6 <= 0) & ~done
        t = d2 / (d2 - d6)
        out[m5] = (a + ac * t[:, None])[m5]
        done |= m5
        m6 = (va <= 0) & ((d4 - d3) >= 0) & ((d5 - d6) >= 0) & ~done
        t = (d4 - d3) / ((d4 - d3) + (d5 - d6))
        out[m6] = (b + (c - b) * t[:, None])[m6]
    return out


@dataclass
class Contact:
    a: int
    b: int
    distance: float  # min surface distance (0 ≈ touching)
    interference: float  # max penetration depth of a's surface into b, mm
    touching_kinds: dict[str, int]  # B-rep face types of a that touch b
    normals: np.ndarray  # (K, 3) a's outward normals where it touches b
    gap_pts: np.ndarray  # (G, 3) points of a within the band of b, not touching


def _overlap(alo, ahi, blo, bhi, pad) -> bool:
    return not (np.any(alo > bhi + pad) or np.any(blo > ahi + pad))


def find_contacts(
    us: list[Unit], targets: list[int] | None = None, tol: float = 0.1, band: float = 1.0
) -> list[Contact]:
    """Contacts from each target unit (default: all) to every other unit within `band`."""
    targets = list(range(len(us))) if targets is None else targets
    out = []
    for a in targets:
        ua = us[a]
        for b, ub in enumerate(us):
            if a == b or not _overlap(ua.lo, ua.hi, ub.lo, ub.hi, band):
                continue
            sa = ua.surface
            near = np.all((sa.pts >= ub.lo - band) & (sa.pts <= ub.hi + band), axis=1)
            if not near.any():
                continue
            idx = np.nonzero(near)[0]
            p = sa.pts[idx]
            d, q, nb = ub.surface.distance(p, band)
            fin = np.isfinite(d)
            # Direction to the obstacle vs. its surface normal. Only trust readings
            # where they're (anti)parallel: next to a convex edge (a hole rim) the
            # nearest point is the edge itself, the direction is oblique, and
            # neither "inside" nor "gap ahead" is meaningful.
            u = np.zeros_like(p)
            far = fin & (d > 1e-6)
            u[far] = (q[far] - p[far]) / d[far, None]
            cos = np.einsum("ij,ij->i", u, nb)
            # Real contact has opposing faces. Near-coincidence at a rim (a head
            # flush with a tight counterbore's edge) isn't, and mustn't constrain.
            opposed = np.einsum("ij,ij->i", sa.pts_n[idx], nb) <= -0.9
            touch = fin & (d <= tol) & opposed
            inside = fin & (d > tol) & (cos >= 0.9)  # interpenetration (modelled threads, press fits)
            gap = fin & (d > tol) & (d <= band) & (cos <= -0.9)
            if not (touch.any() or inside.any() or gap.any()):
                continue
            kinds: dict[str, int] = {}
            for fi in np.unique(sa.mesh.face_of_tri[sa.pts_tri[idx[touch]]]):
                k = sa.face_kind[fi]
                kinds[k] = kinds.get(k, 0) + 1
            pen = d[inside]
            out.append(
                Contact(
                    a, b, float(d.min()), float(pen.max()) if len(pen) else 0.0, kinds,
                    sa.pts_n[idx[touch]], p[gap],
                )
            )
    return out


def _hits(ub: Unit, pts: np.ndarray, tol: float) -> np.ndarray:
    """Which points touch or lie inside unit b (reliable readings only, see find_contacts)."""
    d, q, nb = ub.surface.distance(pts, 2 * tol)
    fin = np.isfinite(d)
    u = np.zeros_like(pts)
    far = fin & (d > 1e-6)
    u[far] = (q[far] - pts[far]) / d[far, None]
    inside = np.einsum("ij,ij->i", u, nb) >= 0.9
    hit = fin & (d <= tol / 2)
    # Deeper than the probe radius: a point that crossed into b between sweep steps.
    d2, q2, nb2 = ub.surface.distance(pts, 5.0)
    f2 = np.isfinite(d2) & (d2 > 1e-6)
    u2 = np.zeros_like(pts)
    u2[f2] = (q2[f2] - pts[f2]) / d2[f2, None]
    return hit | (inside & fin) | (f2 & (np.einsum("ij,ij->i", u2, nb2) >= 0.9) & (d2 > tol))


def blocked(
    c: Contact,
    v: np.ndarray,
    us: list[Unit],
    travel: float = 3.0,
    step: float = 0.5,
    slack: float = 0.03,
    tol: float = 0.1,
    min_hits: int = 3,
) -> bool:
    """Sample noise near edges yields the odd stray point, so a few must agree."""
    if np.count_nonzero(c.normals @ v > slack) >= min_hits:
        return True
    if len(c.gap_pts) == 0:
        return False
    ub = us[c.b]
    for t in np.arange(step, travel + 1e-9, step):
        if np.count_nonzero(_hits(ub, c.gap_pts + t * v, tol)) >= min_hits:
            return True
    return False


def seating(contacts: list[Contact], v: np.ndarray) -> float:
    """How squarely a unit's contact faces push it out along v (1: seated flat, pull straight off)."""
    ns = [c.normals for c in contacts if len(c.normals)]
    if not ns:
        return 0.0
    return float(-(np.concatenate(ns) @ v).mean())


def candidate_dirs(u: Unit) -> list[np.ndarray]:
    """±model axes, then ±principal axes of the unit (a bolt's own axis, even when skewed)."""
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps

    from .geom import v3

    out = [np.array(v, float) for v in np.concatenate([np.eye(3), -np.eye(3)])]
    g = GProp_GProps()
    BRepGProp.VolumeProperties_s(u.shape, g)
    pp = g.PrincipalProperties()
    for ax in (pp.FirstAxisOfInertia(), pp.SecondAxisOfInertia(), pp.ThirdAxisOfInertia()):
        a = v3(ax)
        a /= np.linalg.norm(a)
        for s in (a, -a):
            if all(np.dot(s, o) < 0.999 for o in out):
                out.append(s)
    return out


def free_directions(
    a: int, others: set[int], contacts: list[Contact], us: list[Unit], cands: list[np.ndarray]
) -> list[np.ndarray]:
    """Candidate directions along which unit a slides clear of every unit in `others`."""
    sel = [c for c in contacts if c.a == a and c.b in others]
    return [v for v in cands if not any(blocked(c, v, us) for c in sel)]


@dataclass
class Step:
    unit: int
    direction: np.ndarray  # removal direction; assembly is the reverse
    seated_on: list[int]  # units still present that it touches


def disassemble(us: list[Unit], contacts: list[Contact]) -> tuple[list[Step], list[int]]:
    """Greedy disassembly: repeatedly remove a unit that slides free of everything left.

    Smallest first, so fasteners come out before the parts they hold.
    Returns the removal steps and any units left stuck. Reversed, it's an
    assembly order.
    """
    remaining = set(range(len(us)))
    touching: dict[int, set[int]] = {}
    for c in contacts:
        touching.setdefault(c.a, set()).add(c.b)
    by_a: dict[int, list[Contact]] = {}
    for c in contacts:
        by_a.setdefault(c.a, []).append(c)
    cands = {i: candidate_dirs(us[i]) for i in remaining}
    vol = {i: float(np.prod(us[i].hi - us[i].lo)) for i in remaining}
    cache: dict[tuple[int, int, int], bool] = {}

    def is_blocked(c: Contact, k: int) -> bool:
        key = (c.a, c.b, k)
        if key not in cache:
            cache[key] = blocked(c, cands[c.a][k], us)
        return cache[key]

    steps: list[Step] = []
    while len(remaining) > 1:
        found = None
        for i in sorted(remaining, key=lambda i: vol[i]):
            sel = [c for c in by_a.get(i, []) if c.b in remaining]
            free = [cands[i][k] for k in range(len(cands[i])) if not any(is_blocked(c, k) for c in sel)]
            if free:
                found = (i, max(free, key=lambda v: seating(sel, v)))
                break
        if not found:
            break
        i, d = found
        steps.append(Step(i, d, sorted(touching.get(i, set()) & (remaining - {i}))))
        remaining.remove(i)
    if len(remaining) == 1:
        steps.append(Step(remaining.pop(), np.zeros(3), []))
    return steps, sorted(remaining)
