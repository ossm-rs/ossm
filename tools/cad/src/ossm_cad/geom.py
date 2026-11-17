"""Per-shape geometric analysis on the exact B-rep (not a mesh).

Everything here is in millimetres, in whatever frame the shape is given in.
"""

from __future__ import annotations

import math
from collections import defaultdict
from dataclasses import dataclass, field

import numpy as np
from OCP.Bnd import Bnd_Box
from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepGProp import BRepGProp, BRepGProp_Face
from OCP.BRepTools import BRepTools
from OCP.GeomAbs import GeomAbs_Cylinder, GeomAbs_Plane
from OCP.gp import gp_Pnt, gp_Trsf, gp_Vec
from OCP.GProp import GProp_GProps
from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopLoc import TopLoc_Location
from OCP.TopoDS import TopoDS, TopoDS_Face, TopoDS_Shape


def v3(p) -> np.ndarray:
    return np.array([p.X(), p.Y(), p.Z()])


def transformed(shape: TopoDS_Shape, m: np.ndarray) -> TopoDS_Shape:
    t = gp_Trsf()
    t.SetValues(*m[0, :4], *m[1, :4], *m[2, :4])
    return shape.Moved(TopLoc_Location(t))


def faces(shape: TopoDS_Shape) -> list[TopoDS_Face]:
    out = []
    exp = TopExp_Explorer(shape, TopAbs_FACE)
    while exp.More():
        out.append(TopoDS.Face_s(exp.Current()))
        exp.Next()
    return out


def solid_count(shape: TopoDS_Shape) -> int:
    n = 0
    exp = TopExp_Explorer(shape, TopAbs_SOLID)
    while exp.More():
        n += 1
        exp.Next()
    return n


def bbox(shape: TopoDS_Shape) -> tuple[np.ndarray, np.ndarray]:
    b = Bnd_Box()
    BRepBndLib.Add_s(shape, b, True)
    xmin, ymin, zmin, xmax, ymax, zmax = b.Get()
    return np.array([xmin, ymin, zmin]), np.array([xmax, ymax, zmax])


def face_normal(face: TopoDS_Face) -> tuple[np.ndarray, np.ndarray]:
    """Point and outward (orientation-aware) unit normal at the face's UV centre."""
    umin, umax, vmin, vmax = BRepTools.UVBounds_s(face)
    p, n = gp_Pnt(), gp_Vec()
    BRepGProp_Face(face).Normal((umin + umax) / 2, (vmin + vmax) / 2, p, n)
    nv = v3(n)
    norm = np.linalg.norm(nv)
    return v3(p), nv / norm if norm > 0 else nv


def face_area(face: TopoDS_Face) -> float:
    g = GProp_GProps()
    BRepGProp.SurfaceProperties_s(face, g)
    return g.Mass()


def canon_dir(d: np.ndarray) -> np.ndarray:
    """Pick a sign so parallel/antiparallel axes compare equal."""
    d = d / np.linalg.norm(d)
    for c in d:
        if abs(c) > 1e-6:
            return d if c > 0 else -d
    return d


def axis_label(d: np.ndarray) -> str:
    """'+Z', '-X' for axis-aligned directions, else rounded vector."""
    names = "XYZ"
    i = int(np.argmax(np.abs(d)))
    if abs(abs(d[i]) - 1) < 1e-3:
        return ("+" if d[i] > 0 else "-") + names[i]
    return "[" + ", ".join(f"{c:.3f}" for c in d) + "]"


@dataclass
class CylSegment:
    radius: float
    v0: float  # extent along the axis, measured from `origin`
    v1: float
    coverage_deg: float
    concave: bool  # True: hole/bore, False: boss/shaft


@dataclass
class AxisFeature:
    """Coaxial cylindrical surfaces sharing one axis line (a hole stack or shaft)."""

    id: str
    direction: np.ndarray
    origin: np.ndarray  # point on the axis closest to the part's centre of mass
    segments: list[CylSegment] = field(default_factory=list)
    face_ids: list[int] = field(default_factory=list)  # indices into faces(shape)

    @property
    def kind(self) -> str:
        concave = {s.concave for s in self.segments}
        return "hole" if concave == {True} else "boss" if concave == {False} else "mixed"

    def describe(self) -> str:
        segs = sorted(self.segments, key=lambda s: s.v0)
        parts = [
            f"Ø{2 * s.radius:.2f} {'bore' if s.concave else 'shaft'} {s.v1 - s.v0:.1f}mm"
            + ("" if s.coverage_deg > 350 else f" ({s.coverage_deg:.0f}°)")
            for s in segs
        ]
        hint = size_hint(self)
        return " → ".join(parts) + (f"  [{hint}]" if hint else "")


def size_hint(f: AxisFeature) -> str:
    """Heuristic fastener/bearing identification from bore diameters (printed parts run oversize)."""
    ds = sorted({round(2 * s.radius, 2) for s in f.segments if s.concave})
    hints = []
    for d in ds:
        if 2.4 <= d < 2.95:
            hints.append("M3 tap/self-thread")
        elif 2.95 <= d <= 3.7:
            hints.append("M3 clearance")
        elif 3.9 <= d < 4.9:
            hints.append("M5 tap / heat-set")
        elif 4.9 <= d <= 5.7:
            hints.append("M5 clearance / Ø5 pin")
        elif 5.7 < d <= 6.8:
            hints.append("M3 head counterbore")
        elif 8.0 <= d <= 10.5:
            hints.append("M5 head counterbore")
        elif 15.8 <= d <= 16.6:
            hints.append("625 bearing seat")
    return ", ".join(dict.fromkeys(hints))


@dataclass
class PlaneFeature:
    normal: np.ndarray  # outward
    offset: float  # signed distance of the plane from origin along normal
    area: float
    face_ids: list[int] = field(default_factory=list)


@dataclass
class PartReport:
    name: str
    solids: int
    bbox_min: np.ndarray
    bbox_max: np.ndarray
    volume: float
    area: float
    centre: np.ndarray
    principal_axes: list[np.ndarray]
    principal_moments: list[float]
    face_types: dict[str, int]
    axes: list[AxisFeature]
    planes: list[PlaneFeature]

    @property
    def size(self) -> np.ndarray:
        return self.bbox_max - self.bbox_min


_TYPE_NAMES = {
    0: "plane", 1: "cylinder", 2: "cone", 3: "sphere", 4: "torus",
    5: "bezier", 6: "bspline", 7: "revolution", 8: "extrusion", 9: "offset", 10: "other",
}


def analyse(name: str, shape: TopoDS_Shape) -> PartReport:
    g = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, g)
    volume = g.Mass()
    centre = v3(g.CentreOfMass())
    pp = g.PrincipalProperties()
    moments = list(pp.Moments())
    paxes = [v3(pp.FirstAxisOfInertia()), v3(pp.SecondAxisOfInertia()), v3(pp.ThirdAxisOfInertia())]

    sg = GProp_GProps()
    BRepGProp.SurfaceProperties_s(shape, sg)

    types: dict[str, int] = defaultdict(int)
    cyl_groups: dict[tuple, AxisFeature] = {}
    seg_acc: dict[tuple, dict[tuple, list]] = defaultdict(lambda: defaultdict(list))
    plane_groups: dict[tuple, PlaneFeature] = {}

    for fi, face in enumerate(faces(shape)):
        surf = BRepAdaptor_Surface(face)
        st = surf.GetType()
        types[_TYPE_NAMES.get(int(st), str(st))] += 1

        if st == GeomAbs_Cylinder:
            cyl = surf.Cylinder()
            ax = cyl.Axis()
            d = canon_dir(v3(ax.Direction()))
            o = v3(ax.Location())
            # Axis line point closest to the centre of mass: a stable key for coaxial faces.
            o = o + d * np.dot(centre - o, d)
            r = cyl.Radius()
            key = (tuple(np.round(d, 3)), tuple(np.round(o, 1)))
            feat = cyl_groups.setdefault(key, AxisFeature("", d, o))
            umin, umax, vmin, vmax = BRepTools.UVBounds_s(face)
            # v runs along the cylinder's own axis; re-express against our canonical origin/direction.
            sign = 1.0 if np.dot(v3(ax.Direction()), d) > 0 else -1.0
            base = np.dot(v3(ax.Location()) - o, d)
            a, b = sorted((base + sign * vmin, base + sign * vmax))
            p, n = face_normal(face)
            radial = (p - o) - d * np.dot(p - o, d)
            concave = bool(np.dot(n, radial) < 0)
            seg_acc[key][(round(r, 3), concave)].append((a, b, math.degrees(umax - umin)))
            feat.face_ids.append(fi)
        elif st == GeomAbs_Plane:
            p, n = face_normal(face)
            key = (tuple(np.round(n, 3)), round(float(np.dot(p, n)), 1))
            pf = plane_groups.setdefault(key, PlaneFeature(n, float(np.dot(p, n)), 0.0))
            pf.area += face_area(face)
            pf.face_ids.append(fi)

    axes: list[AxisFeature] = []
    for key, feat in cyl_groups.items():
        for (r, concave), spans in seg_acc[key].items():
            # Fusion splits one bore into several faces (seams, cross-cuts). Cluster
            # faces whose axial spans overlap; angular coverage is their summed sweep.
            spans.sort()
            clusters: list[list[float]] = []  # [v0, v1, coverage]
            for a, b, cov in spans:
                if clusters and a <= clusters[-1][1] + 0.05:
                    clusters[-1][1] = max(clusters[-1][1], b)
                    clusters[-1][2] += cov
                else:
                    clusters.append([a, b, cov])
            for a, b, cov in clusters:
                feat.segments.append(CylSegment(r, a, b, min(cov, 360.0), concave))
        # Drop fillet-only groups: real holes/shafts wrap (nearly) all the way round.
        if any(s.coverage_deg >= 300 for s in feat.segments):
            feat.segments = [s for s in feat.segments if s.coverage_deg >= 170]
            axes.append(feat)

    axes.sort(key=lambda f: (axis_label(f.direction), -max(s.radius for s in f.segments)))
    for i, f in enumerate(axes):
        f.id = f"A{i + 1}"

    planes = sorted(plane_groups.values(), key=lambda p: -p.area)

    lo, hi = bbox(shape)
    return PartReport(
        name=name,
        solids=solid_count(shape),
        bbox_min=lo,
        bbox_max=hi,
        volume=volume,
        area=sg.Mass(),
        centre=centre,
        principal_axes=paxes,
        principal_moments=moments,
        face_types=dict(types),
        axes=axes,
        planes=planes,
    )
