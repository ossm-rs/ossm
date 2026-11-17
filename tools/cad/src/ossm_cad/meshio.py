"""Read print files (STL, 3MF) and fingerprint solids independently of pose.

A fingerprint is (volume, surface area, principal moments of inertia about
the centroid). All three survive rotation and translation, so a part matches
its print file even though the STL is laid out for the print bed rather than
posed as in the assembly.
"""

from __future__ import annotations

import re
import struct
import xml.etree.ElementTree as ET
import zipfile
from dataclasses import dataclass
from pathlib import Path

import numpy as np


@dataclass(frozen=True)
class Fingerprint:
    volume: float  # mm³
    area: float  # mm²
    moments: tuple[float, float, float]  # ascending, unit density, mm⁵

    def to_json(self) -> dict:
        # Significant figures, not decimals: SMD parts are fractions of a mm³.
        return {"volume": float(f"{self.volume:.6g}"), "area": float(f"{self.area:.6g}"), "moments": [float(f"{m:.6g}") for m in self.moments]}

    @classmethod
    def from_json(cls, d: dict) -> "Fingerprint":
        return cls(d["volume"], d["area"], tuple(d["moments"]))

    def distance(self, other: "Fingerprint", use_area: bool = True) -> float:
        """Largest relative difference across the metrics (0.01 = 1%)."""
        pairs = [(self.volume, other.volume)] + list(zip(self.moments, other.moments))
        if use_area:
            pairs.append((self.area, other.area))
        return max(abs(a - b) / max(abs(a), abs(b), 1e-9) for a, b in pairs)


def mesh_fingerprint(tris: np.ndarray) -> Fingerprint:
    """Fingerprint of a closed triangle mesh, (N, 3, 3) vertices, outward winding.

    Signed tetrahedra against the origin (Eberly, "Polyhedral Mass Properties").
    """
    a, b, c = tris[:, 0], tris[:, 1], tris[:, 2]
    cross = np.cross(b - a, c - a)
    area = 0.5 * np.linalg.norm(cross, axis=1).sum()
    det = np.einsum("ij,ij->i", a, np.cross(b, c))  # 6 × signed tet volume
    volume = det.sum() / 6
    if volume < 0:  # inward winding
        det, volume = -det, -volume
    centroid = (det[:, None] * (a + b + c)).sum(axis=0) / (24 * volume)
    # Second moments: ∫ x xᵀ over each tet = det/120 · (Σv Σvᵀ + Σ v vᵀ).
    s = a + b + c
    cov = np.einsum("i,ij,ik->jk", det, s, s) + sum(np.einsum("i,ij,ik->jk", det, v, v) for v in (a, b, c))
    cov /= 120
    cov -= volume * np.outer(centroid, centroid)
    inertia = np.trace(cov) * np.eye(3) - cov
    moments = np.sort(np.linalg.eigvalsh(inertia))
    return Fingerprint(float(volume), float(area), tuple(float(m) for m in moments))


def read_stl(path: Path) -> np.ndarray:
    data = path.read_bytes()
    if len(data) >= 84:
        (n,) = struct.unpack_from("<I", data, 80)
        if 84 + 50 * n == len(data):
            rec = np.frombuffer(data, dtype=np.dtype([("n", "<f4", 3), ("v", "<f4", (3, 3)), ("a", "<u2")]), count=n, offset=84)
            return rec["v"].astype(np.float64)
    verts = re.findall(rb"vertex\s+(\S+)\s+(\S+)\s+(\S+)", data)
    return np.array(verts, dtype=np.float64).reshape(-1, 3, 3)


def _transform(attr: str | None) -> np.ndarray:
    m = np.eye(4)
    if attr:
        v = [float(x) for x in attr.split()]
        # 3MF: 12 values, row-major 4x3 (last column implied), applied as row vectors.
        m[:3, :3] = np.array(v[:9]).reshape(3, 3).T
        m[:3, 3] = v[9:12]
    return m


def read_3mf(path: Path) -> np.ndarray:
    """All triangles placed by the build items, in mm."""
    ns = {"m": "http://schemas.microsoft.com/3dmanufacturing/core/2015/02"}
    scale = {"micron": 0.001, "millimeter": 1.0, "centimeter": 10.0, "inch": 25.4, "foot": 304.8, "meter": 1000.0}
    with zipfile.ZipFile(path) as z:
        models = {"/" + n: ET.fromstring(z.read(n)) for n in z.namelist() if n.endswith(".model")}
    objects: dict[tuple[str, str], ET.Element] = {}
    for name, root in models.items():
        for obj in root.iter(f"{{{ns['m']}}}object"):
            objects[(name, obj.get("id"))] = obj
    main = next(n for n in models if n.lower().endswith("3d/3dmodel.model"))
    unit = scale.get(models[main].get("unit", "millimeter"), 1.0)
    out: list[np.ndarray] = []

    def emit(model: str, oid: str, m: np.ndarray, depth: int = 0):
        obj = objects.get((model, oid))
        if obj is None or depth > 8:
            return
        mesh = obj.find("m:mesh", ns)
        if mesh is not None:
            v = np.array([[float(x.get(k)) for k in "xyz"] for x in mesh.find("m:vertices", ns)])
            t = np.array([[int(x.get(k)) for k in ("v1", "v2", "v3")] for x in mesh.find("m:triangles", ns)])
            if len(v) and len(t):
                vh = np.c_[v, np.ones(len(v))] @ m.T
                out.append(vh[:, :3][t])
        comps = obj.find("m:components", ns)
        if comps is not None:
            for c in comps:
                ref = c.get("{http://schemas.microsoft.com/3dmanufacturing/production/2015/06}path") or model
                emit(ref, c.get("objectid"), m @ _transform(c.get("transform")), depth + 1)

    for item in models[main].iter(f"{{{ns['m']}}}item"):
        emit(main, item.get("objectid"), _transform(item.get("transform")))
    return np.concatenate(out) * unit if out else np.zeros((0, 3, 3))


def read_mesh(path: Path) -> np.ndarray:
    return read_3mf(path) if path.suffix.lower() == ".3mf" else read_stl(path)
