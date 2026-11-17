"""Parts the upstream assembly doesn't model, derived from its geometry.

- Nuts: printed parts have hex pockets (or side-entry slots) cut around bolt
  holes. Each pocket gets a standard nut, oriented to its flats and seated at
  the end facing the bolt's clearance bore.
- Authored additions (variants.json `additions`): standard bolts, a part from
  another upstream STEP, and the GT2 belt, whose path is computed from the
  pulley, the idler bearings and the clamp ends in the index.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
from OCP.BRep import BRep_Tool
from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.GeomAbs import GeomAbs_Plane
from OCP.TopAbs import TopAbs_VERTEX
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS, TopoDS_Shape

from .geom import analyse, face_normal, faces, v3

# ISO 4032 hex nuts: across-flats, thickness. Pockets run a little oversize.
NUTS = {3: (5.5, 2.4), 4: (7.0, 3.2), 5: (8.0, 4.0), 6: (10.0, 5.2)}
# ISO 4762 socket head cap screws: head diameter, head height = d.
SHCS_HEAD = {3: 5.5, 4: 7.0, 5: 8.5, 6: 10.0}


def _verts(face) -> np.ndarray:
    out = []
    exp = TopExp_Explorer(face, TopAbs_VERTEX)
    while exp.More():
        out.append(v3(BRep_Tool.Pnt_s(TopoDS.Vertex_s(exp.Current()))))
        exp.Next()
    return np.array(out).reshape(-1, 3)


def _size_for_af(af: float) -> int | None:
    for d, (nominal, _) in NUTS.items():
        if nominal - 0.2 <= af <= nominal + 0.7:
            return d
    return None


@dataclass
class Pocket:
    hole: str  # hole feature id within the part (A3, …)
    size: int  # metric thread size the nut fits
    frame: np.ndarray  # 4x4, part-local: origin = nut centre, z = hole axis, x = a flat's normal


def nut_pockets(shape: TopoDS_Shape) -> list[Pocket]:
    """Hex pockets (≥3 flats 60° apart) and side-entry slots (2 opposite flats) around holes."""
    planes = []
    for f in faces(shape):
        if BRepAdaptor_Surface(f).GetType() == GeomAbs_Plane:
            p, n = face_normal(f)
            planes.append((p, n, _verts(f)))
    out = []
    for ax in analyse("", shape).axes:
        if not any(s.concave for s in ax.segments):
            continue
        d, o = ax.direction, ax.origin
        ref = np.cross(d, [1, 0, 0] if abs(d[0]) < 0.9 else [0, 1, 0])
        ref /= np.linalg.norm(ref)
        cands = []
        for p, n, v in planes:
            if abs(np.dot(n, d)) > 0.02 or len(v) == 0:
                continue
            r = float(np.dot(n, o - p))  # > 0: the face looks at the axis (a pocket wall)
            if not 2.5 < r < 5.5:
                continue
            rel = v - o
            rad = np.linalg.norm(rel - np.outer(rel @ d, d), axis=1)
            if rad.min() > 1.6 * r + 0.5:  # hex corners sit at 1.15r, slot walls further out
                continue
            ang = math.degrees(math.atan2(np.dot(np.cross(ref, n), d), np.dot(ref, n))) % 360
            cands.append((r, ang, n, v @ d))
        cands.sort(key=lambda c: c[0])
        while cands:
            r0 = cands[0][0]
            grp = [c for c in cands if abs(c[0] - r0) < 0.15]
            cands = [c for c in cands if abs(c[0] - r0) >= 0.15]
            base = grp[0][1]
            flats = [c for c in grp if min((c[1] - base) % 60, 60 - (c[1] - base) % 60) < 2]
            sectors = {round(((c[1] - base) % 360) / 60) % 6 for c in flats}
            opposite = any((s + 3) % 6 in sectors for s in sectors)
            af = 2 * float(np.mean([c[0] for c in flats]))
            size = _size_for_af(af)
            if size is None or not (len(sectors) >= 3 or (len(sectors) == 2 and opposite)):
                continue
            # The hole must take that thread (tap to clearance size); this rejects
            # hex recesses around dowels and pin holes.
            if not any(s.concave and 0.4 * size <= s.radius <= 0.6 * size + 0.4 for s in ax.segments):
                continue
            along = np.concatenate([c[3] for c in flats])
            lo, hi = float(along.min()), float(along.max())
            m = NUTS[size][1]
            if hi - lo <= m + 0.8:
                centre = (lo + hi) / 2
            else:
                # Long pocket (an insertion channel): the nut sits at the end facing the bore.
                base_v = float(np.dot(o, d))
                bores = [base_v + (s.v0 + s.v1) / 2 for s in ax.segments if s.concave and s.radius < r0]
                near_lo = not bores or min(abs(b - lo) for b in bores) <= min(abs(b - hi) for b in bores)
                centre = lo + m / 2 if near_lo else hi - m / 2
            x = flats[0][2]
            frame = np.eye(4)
            frame[:3, 0] = x
            frame[:3, 1] = np.cross(d, x)
            frame[:3, 2] = d
            frame[:3, 3] = o + d * (centre - float(np.dot(o, d)))
            # Near-coaxial hole features can see the same pocket twice.
            if not any(q.size == size and np.linalg.norm(q.frame[:3, 3] - frame[:3, 3]) < 1.5 for q in out):
                out.append(Pocket(ax.id, size, frame))
    return out


def hex_nut(size: int) -> TopoDS_Shape:
    """Nut centred on the origin, axis +Z, one flat facing +X."""
    from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut
    from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeFace, BRepBuilderAPI_MakePolygon
    from OCP.BRepPrimAPI import BRepPrimAPI_MakeCylinder, BRepPrimAPI_MakePrism
    from OCP.gp import gp_Ax2, gp_Dir, gp_Pnt, gp_Vec

    af, m = NUTS[size]
    rc = af / math.sqrt(3)  # corner radius
    poly = BRepBuilderAPI_MakePolygon()
    for k in range(6):
        a = math.radians(30 + 60 * k)
        poly.Add(gp_Pnt(rc * math.cos(a), rc * math.sin(a), -m / 2))
    poly.Close()
    body = BRepPrimAPI_MakePrism(BRepBuilderAPI_MakeFace(poly.Wire()).Face(), gp_Vec(0, 0, m)).Shape()
    bore = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, -m), gp_Dir(0, 0, 1)), size / 2, 2 * m).Shape()
    return BRepAlgoAPI_Cut(body, bore).Shape()


def shcs(d: int, length: float) -> TopoDS_Shape:
    """Socket head cap screw: head top at the origin, shank along +Z (the insertion direction)."""
    from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut, BRepAlgoAPI_Fuse
    from OCP.BRepPrimAPI import BRepPrimAPI_MakeCylinder
    from OCP.gp import gp_Ax2, gp_Dir, gp_Pnt

    head_d = SHCS_HEAD[d]
    head = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), head_d / 2, d).Shape()
    shank = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, d), gp_Dir(0, 0, 1)), d / 2, length).Shape()
    socket = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, -0.1), gp_Dir(0, 0, 1)), d * 0.45, d * 0.6).Shape()
    return BRepAlgoAPI_Cut(BRepAlgoAPI_Fuse(head, shank).Shape(), socket).Shape()


def frame_from_axis(origin, direction) -> np.ndarray:
    z = np.asarray(direction, float)
    z /= np.linalg.norm(z)
    x = np.cross(z, [0, 0, 1] if abs(z[2]) < 0.9 else [1, 0, 0])
    x /= np.linalg.norm(x)
    m = np.eye(4)
    m[:3, 0], m[:3, 1], m[:3, 2], m[:3, 3] = x, np.cross(z, x), z, origin
    return m


# --- belt ---------------------------------------------------------------------


@dataclass
class Pulley:
    centre: np.ndarray  # 2D, in the belt plane
    radius: float  # to the belt's centreline
    ccw: bool  # wrap direction as the belt travels


def _tangent(a: Pulley | np.ndarray, b: Pulley | np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Belt run leaving `a` and arriving on `b` (either may be a bare point)."""

    def as_circle(c):
        return (c.centre, c.radius, c.ccw) if isinstance(c, Pulley) else (np.asarray(c, float), 0.0, True)

    ca, ra, sa = as_circle(a)
    cb, rb, sb = as_circle(b)
    # Signed radii: the belt sits on the left of a ccw wrap, right of a cw wrap.
    Ra, Rb = (ra if sa else -ra), (rb if sb else -rb)
    v = cb - ca
    dist = float(np.linalg.norm(v))
    u = v / dist
    perp = np.array([-u[1], u[0]])
    k = (Ra - Rb) / dist
    h = math.sqrt(max(0.0, 1 - k * k))
    # Normal n (pointing from centre to tangent point, scaled by the signed radius).
    n = -k * u + h * perp
    return ca - Ra * n, cb - Rb * n


def _arc(p: Pulley, start: np.ndarray, end: np.ndarray, step_deg: float = 6) -> list[np.ndarray]:
    a0 = math.atan2(*(start - p.centre)[::-1])
    a1 = math.atan2(*(end - p.centre)[::-1])
    sweep = (a1 - a0) % (2 * math.pi) if p.ccw else -((a0 - a1) % (2 * math.pi))
    n = max(2, int(abs(math.degrees(sweep)) / step_deg))
    return [p.centre + p.radius * np.array([math.cos(a0 + sweep * i / n), math.sin(a0 + sweep * i / n)]) for i in range(n + 1)]


def belt_path(points: list) -> np.ndarray:
    """Centreline through end points and pulleys, in 2D: [start, Pulley…, end]."""
    out: list[np.ndarray] = [np.asarray(points[0], float)]
    arrive = None
    for a, b in zip(points, points[1:]):
        leave, land = _tangent(a, b)
        if isinstance(a, Pulley) and arrive is not None:
            out += _arc(a, arrive, leave)[1:]
        out.append(leave)
        out.append(land)
        arrive = land
    return np.array(out)


def with_tails(path2d: np.ndarray, length: float | None) -> np.ndarray:
    """A belt of `length` along `path2d`: the spare length is split between the two
    tips, each carried straight on from its last run."""
    if not length:
        return path2d
    spare = length - float(np.sum(np.linalg.norm(np.diff(path2d, axis=0), axis=1)))
    if spare <= 0:
        return path2d
    out = path2d.copy()
    for tip, step in ((0, 1), (len(out) - 1, -1)):
        # The run into the tip: from the nearest point that isn't the tip itself.
        prev = tip + step
        while 0 <= prev < len(out) and np.linalg.norm(out[tip] - out[prev]) < 1e-9:
            prev += step
        d = out[tip] - out[prev]
        out[tip] = out[tip] + d / np.linalg.norm(d) * spare / 2
    return out


def belt_mesh(path2d: np.ndarray, plane: np.ndarray, width: float, thickness: float):
    """Extrude a 2D centreline into a flat belt. `plane`: 4x4, 2D (x, y) → 3D, belt width along z."""
    pts = np.c_[path2d, np.zeros(len(path2d)), np.ones(len(path2d))] @ plane.T
    pts = pts[:, :3]
    keep = np.r_[True, np.linalg.norm(np.diff(pts, axis=0), axis=1) > 1e-6]
    pts = pts[keep]
    z = plane[:3, 2]
    tang = np.gradient(pts, axis=0)
    tang /= np.linalg.norm(tang, axis=1, keepdims=True)
    nrm = np.cross(z, tang)
    rings = []
    for s_n, s_z in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        rings.append(pts + nrm * (s_n * thickness / 2) + z * (s_z * width / 2))
    verts = np.concatenate(rings)
    n = len(pts)
    tris = []
    for k in range(4):
        a, b = k * n, ((k + 1) % 4) * n
        for i in range(n - 1):
            tris += [[a + i, b + i, b + i + 1], [a + i, b + i + 1, a + i + 1]]
    # End caps.
    for e in (0, n - 1):
        q = [k * n + e for k in range(4)]
        tris += [[q[0], q[2], q[1]], [q[0], q[3], q[2]]] if e == 0 else [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]
    return verts, np.array(tris)
