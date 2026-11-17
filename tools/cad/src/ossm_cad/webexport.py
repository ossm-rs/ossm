"""Export a variant's index as a lightweight GLB for the web demos.

Demo fidelity, not print fidelity: parts are tessellated coarsely (relative to
their size), each unique part is stored once and placed by instance nodes, and
nodes are named by index instance id so the site can refer to them.

Layout of the output scene, all in the configured display frame (metres):

    <group>            one node per configured group (e.g. "housing", "rail")
      <instance id>    e.g. "shcs-m5x30#2": matrix = frame · assembly transform
      <merged unit>    rigid sub-assemblies listed under "merge" become one node
      <derived>        parts the index derives or adds (nuts, extra bolts, belt)
"""

from __future__ import annotations

import json
import re
import struct
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

from .index import brep_fingerprint
from .meshio import Fingerprint
from .step import load

# Base colours by index kind (the site may override per part).
MATERIALS = {
    "printed": ([0.42, 0.11, 0.6, 1.0], 0.0, 0.85),
    "fastener": ([0.62, 0.63, 0.66, 1.0], 0.9, 0.35),
    "hardware": ([0.55, 0.56, 0.6, 1.0], 0.8, 0.35),
    "electronics": ([0.08, 0.3, 0.17, 1.0], 0.1, 0.6),
    "belt": ([0.05, 0.05, 0.05, 1.0], 0.0, 0.9),
}


@dataclass
class MeshData:
    positions: np.ndarray  # (N, 3) float32, part-local mm
    normals: np.ndarray  # (N, 3) float32
    indices: np.ndarray  # (3T,) uint
    material: int | np.ndarray  # one material, or one per triangle (written as a primitive each)


@dataclass
class Node:
    name: str
    matrix: np.ndarray | None = None  # 4x4
    mesh: int | None = None
    children: list[int] = field(default_factory=list)


def tessellate_demo(shape, rel: float = 0.008, min_defl: float = 0.1, angular: float = 0.6):
    """Coarse mesh: deflection scales with part size; normals smooth within each B-rep face.

    Returns positions, normals, flat indices and the B-rep face index of each triangle.
    """
    from OCP.BRep import BRep_Tool
    from OCP.BRepMesh import BRepMesh_IncrementalMesh
    from OCP.BRepTools import BRepTools
    from OCP.TopAbs import TopAbs_REVERSED
    from OCP.TopLoc import TopLoc_Location

    from .geom import bbox, faces

    lo, hi = bbox(shape)
    BRepTools.Clean_s(shape)
    BRepMesh_IncrementalMesh(shape, max(min_defl, rel * float(np.linalg.norm(hi - lo))), False, angular, True)
    pos, nrm, idx, fot = [], [], [], []
    base = 0
    for fi, face in enumerate(faces(shape)):
        loc = TopLoc_Location()
        tri = BRep_Tool.Triangulation_s(face, loc)
        if tri is None:
            continue
        trsf = loc.Transformation()
        p = np.array([[q.X(), q.Y(), q.Z()] for q in (tri.Node(i + 1).Transformed(trsf) for i in range(tri.NbNodes()))])
        t = np.array([tri.Triangle(i + 1).Get() for i in range(tri.NbTriangles())], dtype=np.int64) - 1
        if face.Orientation() == TopAbs_REVERSED:
            t = t[:, [0, 2, 1]]
        # Area-weighted vertex normals within the face: smooth across the face, sharp at its edges.
        fn = np.cross(p[t[:, 1]] - p[t[:, 0]], p[t[:, 2]] - p[t[:, 0]])
        n = np.zeros_like(p)
        for k in range(3):
            np.add.at(n, t[:, k], fn)
        n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-12)
        pos.append(p)
        nrm.append(n)
        idx.append(t + base)
        fot.append(np.full(len(t), fi))
        base += len(p)
    if not pos:
        return np.zeros((0, 3)), np.zeros((0, 3)), np.zeros(0, dtype=np.int64), np.zeros(0, dtype=np.int64)
    return np.concatenate(pos), np.concatenate(nrm), np.concatenate(idx).ravel(), np.concatenate(fot)


def box_mesh(shape) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """The shape's bounding box as a flat-shaded box (stand-in for tiny detailed parts)."""
    from .geom import bbox

    lo, hi = bbox(shape)
    p, n, i = [], [], []
    for axis in range(3):
        for side in (0, 1):
            normal = np.zeros(3)
            normal[axis] = 1 if side else -1
            u, v = (axis + 1) % 3, (axis + 2) % 3
            quad = []
            for a, b in ((0, 0), (1, 0), (1, 1), (0, 1)):
                c = np.empty(3)
                c[axis] = hi[axis] if side else lo[axis]
                c[u] = hi[u] if a else lo[u]
                c[v] = hi[v] if b else lo[v]
                quad.append(c)
            base = len(p)
            p += quad
            n += [normal] * 4
            tri = [[0, 1, 2], [0, 2, 3]] if side else [[0, 2, 1], [0, 3, 2]]
            i += [base + k for t in tri for k in t]
    return np.array(p), np.array(n), np.array(i)


def match_instances(index: dict, asm) -> list[tuple[dict, object]]:
    """Pair index instance rows with the STEP's placed solids (by part fingerprint + transform)."""
    from .contacts import dedupe

    leaves, _ = dedupe(asm.leaf_instances())
    fps: dict[str, Fingerprint] = {}
    parts = {pid: Fingerprint.from_json(p["fingerprint"]) for pid, p in index["parts"].items()}
    rows = {r["id"]: r for r in index["instances"]}
    by_part: dict[str, list[dict]] = {}
    for r in index["instances"]:
        by_part.setdefault(r["part"], []).append(r)
    out = []
    for inst in leaves:
        if inst.proto.entry not in fps:
            fps[inst.proto.entry] = brep_fingerprint(inst.proto.shape)
        fp = fps[inst.proto.entry]
        cands = [r for pid, pf in parts.items() if pf.distance(fp) < 1e-3 for r in by_part.get(pid, [])]
        best = min(cands, key=lambda r: np.abs(np.array(r["transform"]) - inst.world[:3]).max(), default=None)
        if best is None or np.abs(np.array(best["transform"]) - inst.world[:3]).max() > 0.01:
            raise RuntimeError(f"no index instance for {inst.path}; re-run `just cad index`")
        out.append((rows[best["id"]], inst))
    return out


class GLB:
    def __init__(self):
        self.nodes: list[Node] = []
        self.meshes: list[MeshData] = []
        self.materials: list[dict] = []
        self.mesh_names: list[str] = []

    def material(self, name: str, rgba, metallic: float, roughness: float) -> int:
        for i, m in enumerate(self.materials):
            if m["name"] == name:
                return i
        self.materials.append({
            "name": name,
            "pbrMetallicRoughness": {"baseColorFactor": [round(c, 4) for c in rgba], "metallicFactor": metallic, "roughnessFactor": roughness},
        })
        return len(self.materials) - 1

    def mesh(self, name: str, data: MeshData) -> int:
        self.meshes.append(data)
        self.mesh_names.append(name)
        return len(self.meshes) - 1

    def node(self, node: Node) -> int:
        self.nodes.append(node)
        return len(self.nodes) - 1

    def write(self, path: Path, roots: list[int]) -> None:
        blob = bytearray()
        views, accessors = [], []

        def add(arr: np.ndarray, target: int, comp: int, typ: str, minmax: bool = False, normalized: bool = False, stride: int = 0) -> int:
            while len(blob) % 4:
                blob.append(0)
            view = {"buffer": 0, "byteOffset": len(blob), "byteLength": arr.nbytes, "target": target}
            if stride:
                view["byteStride"] = stride
            views.append(view)
            blob.extend(arr.tobytes())
            acc = {"bufferView": len(views) - 1, "componentType": comp, "count": int(arr.shape[0]), "type": typ}
            if normalized:
                acc["normalized"] = True
            if minmax:
                acc["min"] = [float(x) for x in arr.min(axis=0)]
                acc["max"] = [float(x) for x in arr.max(axis=0)]
            accessors.append(acc)
            return len(accessors) - 1

        gl_meshes = []
        for name, m in zip(self.mesh_names, self.meshes):
            pos = add(m.positions.astype(np.float32), 34962, 5126, "VEC3", minmax=True)
            # Normals as normalised int8 (KHR_mesh_quantization), padded to 4-byte vertices.
            q = np.zeros((len(m.normals), 4), np.int8)
            q[:, :3] = np.clip(np.round(m.normals * 127), -127, 127)
            nrm = add(q, 34962, 5120, "VEC3", normalized=True, stride=4)
            big = len(m.positions) > 65535
            tris = m.indices.reshape(-1, 3)
            per_tri = np.broadcast_to(np.asarray(m.material), (len(tris),))
            prims = []
            for mat in np.unique(per_tri):
                sel = tris[per_tri == mat].ravel()
                ind = add(sel.astype(np.uint32 if big else np.uint16), 34963, 5125 if big else 5123, "SCALAR")
                prims.append({"attributes": {"POSITION": pos, "NORMAL": nrm}, "indices": ind, "material": int(mat)})
            gl_meshes.append({"name": name, "primitives": prims})

        gl_nodes = []
        for n in self.nodes:
            d: dict = {"name": n.name}
            if n.matrix is not None and not np.allclose(n.matrix, np.eye(4)):
                d["matrix"] = [float(f"{x:.7g}") for x in n.matrix.T.ravel()]  # column-major
            if n.mesh is not None:
                d["mesh"] = n.mesh
            if n.children:
                d["children"] = n.children
            gl_nodes.append(d)

        while len(blob) % 4:
            blob.append(0)
        doc = {
            "asset": {"version": "2.0", "generator": "ossm-cad webexport"},
            "extensionsUsed": ["KHR_mesh_quantization"],
            "extensionsRequired": ["KHR_mesh_quantization"],
            "scene": 0,
            "scenes": [{"nodes": roots}],
            "nodes": gl_nodes,
            "meshes": gl_meshes,
            "materials": self.materials,
            "accessors": accessors,
            "bufferViews": views,
            "buffers": [{"byteLength": len(blob)}],
        }
        js = json.dumps(doc, separators=(",", ":")).encode()
        js += b" " * (-len(js) % 4)
        out = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(blob))
        out += struct.pack("<II", len(js), 0x4E4F534A) + js
        out += struct.pack("<II", len(blob), 0x004E4942) + bytes(blob)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(out)


# Colours for derived shapes, which have no upstream colour (linear RGB).
SHAPE_COLORS = {"hex-nut": (0.597, 0.597, 0.597), "shcs": (0.352, 0.352, 0.352)}


class Palette:
    """Materials keyed by (kind, colour). A printed part's main colour becomes the
    shared `printed` material (the site themes it); other colours stay as accents."""

    def __init__(self, glb: "GLB"):
        self.glb = glb
        self.by_key: dict[tuple, int] = {}

    def get(self, kind: str, rgb: tuple | None) -> int:
        if rgb is None:
            return self.glb.material(kind, *MATERIALS[kind])
        key = (kind, tuple(round(c, 3) for c in rgb))
        if key not in self.by_key:
            _, metal, rough = MATERIALS[kind]
            # Named by sRGB hex (what a colour picker shows); the factor itself stays linear.
            srgb = [c * 12.92 if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055 for c in key[1]]
            hexed = "".join(f"{int(round(c * 255)):02x}" for c in srgb)
            self.by_key[key] = self.glb.material(f"{kind} #{hexed}", [*key[1], 1.0], metal, rough)
        return self.by_key[key]

    def triangles(self, kind: str, tri_colors: list, weights: np.ndarray | None = None, themed: bool = True) -> np.ndarray:
        """Material per triangle from per-triangle colours (None = uncoloured)."""
        theme = None
        if kind == "printed" and themed:
            # The colour covering most of the part is the printed filament colour.
            keys = [tuple(c) if c is not None else None for c in tri_colors]
            w = np.ones(len(keys)) if weights is None else weights
            tally: dict = {}
            for k, wk in zip(keys, w):
                tally[k] = tally.get(k, 0) + wk
            theme = max(tally, key=tally.get) if tally else None
        out = np.empty(len(tri_colors), dtype=np.int64)
        cache: dict = {}
        for i, c in enumerate(tri_colors):
            k = tuple(c) if c is not None else None
            if k not in cache:
                cache[k] = self.get(kind, None) if (k is None or k == theme) else self.get(kind, k)
            out[i] = cache[k]
        return out


def load_appearance(path: Path | None) -> dict:
    data = json.loads(path.read_text()) if path and path.exists() else {}
    return {k: v for k, v in data.items() if not k.startswith("_")}


def face_colors_with_appearance(colors: list, pid: str, part: dict, appearance: dict, log) -> list:
    """Fill faces upstream left uncoloured from the authored appearance file."""
    a = appearance.get(pid)
    if not a:
        return colors
    if abs(a["volume"] - part["fingerprint"]["volume"]) > 1e-3 * max(1.0, a["volume"]):
        log(f"  appearance for {pid} is for a different revision of the part; ignored")
        return colors
    by_face = {}
    for rgb, idxs in a.get("faces", []):
        for i in idxs:
            by_face[i] = tuple(rgb)
    default = tuple(a["default"]) if a.get("default") else None
    if a.get("theme") is False:  # an authored look for the whole part
        return [by_face.get(i, default) for i in range(len(colors))]
    return [c if c is not None else by_face.get(i, default) for i, c in enumerate(colors)]


def uncap(shape, colors: list, spec: dict, part: dict, pid: str, log):
    """Open holes that the model shows capped (e.g. a rail's plastic hole caps).

    A cap is the slab between two coaxial disc faces of `spec["color"]`; each one
    is cut out with a cylinder of the disc's radius. Returns the new shape and
    its face colours (faces carried over keep theirs; new walls take the colour
    of the faces they cut into).
    """
    import math

    from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut
    from OCP.BRepPrimAPI import BRepPrimAPI_MakeCylinder
    from OCP.GeomAbs import GeomAbs_Plane
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.gp import gp_Ax2, gp_Dir, gp_Pnt
    from OCP.TopTools import TopTools_ListOfShape

    from .geom import face_area, face_normal, faces

    if abs(spec["volume"] - part["fingerprint"]["volume"]) > 1e-3 * max(1.0, spec["volume"]):
        log(f"  uncap for {pid} is for a different revision of the part; ignored")
        return shape, colors
    want = np.array(spec["color"])
    fs = faces(shape)
    discs = []
    for i, f in enumerate(fs):
        c = colors[i] if i < len(colors) else None
        if c is None or np.abs(np.array(c) - want).max() > 0.02:
            continue
        if BRepAdaptor_Surface(f).GetType() != GeomAbs_Plane:
            continue
        p, n = face_normal(f)
        discs.append((p, n, math.sqrt(face_area(f) / math.pi)))
    tools = TopTools_ListOfShape()
    caps = 0
    for a, (pa, na, ra) in enumerate(discs):
        for pb, nb, rb in discs[a + 1:]:
            gap = pb - pa
            # Opposite faces of one slab: facing away from each other, on one axis.
            if np.dot(na, nb) > -0.99 or np.linalg.norm(np.cross(gap, na)) > 0.05 or abs(ra - rb) > 0.05:
                continue
            outer, inner = (pa, pb) if np.dot(gap, na) < 0 else (pb, pa)
            axis = (outer - inner) / np.linalg.norm(outer - inner)
            start = inner - axis * 0.01
            height = float(np.linalg.norm(outer - inner)) + 0.11  # a little past both faces
            cyl = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(*map(float, start)), gp_Dir(*map(float, axis))), ra, height)
            tools.Append(cyl.Shape())
            caps += 1
    if not caps:
        log(f"  uncap for {pid}: no caps found")
        return shape, colors
    args = TopTools_ListOfShape()
    args.Append(shape)
    cut = BRepAlgoAPI_Cut()
    cut.SetArguments(args)
    cut.SetTools(tools)
    cut.Build()
    out = cut.Shape()
    # Carry colours over through the boolean's history.
    source = {}
    for i, f in enumerate(fs):
        mods = cut.Modified(f)
        if mods.Size():
            for m in mods:
                source.setdefault(m, i)
        elif not cut.IsDeleted(f):
            source.setdefault(f, i)
    wall = spec.get("wall")
    new_colors = []
    for f in faces(out):
        i = next((i for k, i in source.items() if k.IsSame(f)), None)
        new_colors.append(colors[i] if i is not None else (tuple(wall) if wall else None))
    log(f"  uncapped {caps} holes in {pid}")
    return out, new_colors


def assembly_mesh(placed: list[tuple[object, np.ndarray, list]], palette: "Palette", kind: str):
    """One mesh for a rigid bought-in assembly, in its parent's coordinates.

    Boards keep their outline; components on them become boxes, coloured like the
    component. Demo fidelity: a KiCad export models every pin, which is most of the
    triangles. `placed`: (shape, world, face colours).
    """
    ps, ns, ix, mats = [], [], [], []
    base = 0
    for shape, w, colors in placed:
        size = np.ptp(np.array(_bbox_pts(shape)), axis=0)
        if size.max() > 30:
            p, n, i, fot = tessellate_demo(shape)
            tri_colors = [colors[f] if f < len(colors) else None for f in fot]
        else:
            p, n, i = box_mesh(shape)
            known = [c for c in colors if c is not None]
            main = max(set(known), key=known.count) if known else None
            tri_colors = [main] * (len(i) // 3)
        ps.append(p @ w[:3, :3].T + w[:3, 3])
        ns.append(n @ w[:3, :3].T)
        ix.append(i + base)
        mats.append(palette.triangles(kind, tri_colors))
        base += len(p)
    return np.concatenate(ps), np.concatenate(ns), np.concatenate(ix), np.concatenate(mats)


def shape_mesh(spec: dict, kind: str, repo: Path, palette: "Palette"):
    """Geometry for parts the index derives or adds (see derived.py), in part coordinates."""
    from .derived import belt_mesh, hex_nut, shcs

    t = spec["type"]
    if t in ("hex-nut", "shcs"):
        solid = hex_nut(spec["size"]) if t == "hex-nut" else shcs(spec["d"], spec["length"])
        p, n, i, _ = tessellate_demo(solid)
        return p, n, i, palette.get(kind, SHAPE_COLORS[t])
    if t == "step":
        asm = load(repo / spec["file"])
        return assembly_mesh([(x.proto.shape, x.world, asm.face_colors(x)) for x in asm.leaf_instances()], palette, kind)
    if t == "belt":
        plane = np.vstack([np.array(spec["plane"]), [0, 0, 0, 1]])
        path = (np.array(spec["path"]) - plane[:3, 3]) @ plane[:3, :2]
        v, tri = belt_mesh(path, plane, spec["width"], spec["thickness"])
        a, b, c = v[tri[:, 0]], v[tri[:, 1]], v[tri[:, 2]]
        if np.einsum("ij,ij->i", a, np.cross(b, c)).sum() < 0:  # keep faces pointing outward
            tri = tri[:, [0, 2, 1]]
        fn = np.cross(v[tri[:, 1]] - v[tri[:, 0]], v[tri[:, 2]] - v[tri[:, 0]])
        n = np.zeros_like(v)
        for k in range(3):
            np.add.at(n, tri[:, k], fn)
        n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-12)
        return v, n, tri.ravel(), palette.get(kind, None)
    raise ValueError(f"unknown shape {t!r}")


def belt_data(node: str, shape: dict, frame: np.ndarray) -> dict:
    """The belt's route for the site, in the display frame (metres): a plane, and in
    it the anchors and wrap circles the centreline runs between (see derived.belt_path)."""
    plane = np.vstack([np.array(shape["plane"]), [0, 0, 0, 1]])
    scale = float(np.cbrt(abs(np.linalg.det(frame[:3, :3]))))  # mm → m
    axis = lambda v: [_round(c) for c in (frame[:3, :3] @ v) / np.linalg.norm(frame[:3, :3] @ v)]
    m = lambda q: [_round(c * scale) for c in q]
    route = []
    for el in shape["route"]:
        if "anchor" in el:
            route.append({"anchor": el["anchor"], "at": m(el["at"])})
        else:
            route.append({"wrap": el["wrap"], "centre": m(el["centre"]), "radius": _round(el["radius"] * scale),
                          "ccw": el["ccw"]})
    return {
        "node": node,
        "plane": {
            "origin": [_round(c) for c in (frame @ plane[:, 3])[:3]],
            "x": axis(plane[:3, 0]), "y": axis(plane[:3, 1]), "z": axis(plane[:3, 2]),
        },
        "width": _round(shape["width"] * scale),
        "thickness": _round(shape["thickness"] * scale),
        "length": _round(shape["length"] * scale),
        "route": route,
        "leads": shape.get("lead_counts", {}),
    }


def _round(x: float) -> float:
    v = round(float(x), 7)
    return 0.0 if v == 0 else v


def catalog_data(index: dict, instances: dict[str, str], belt: dict | None = None) -> dict:
    """What the site needs from the index for the exported nodes: part per instance,
    and per part its name, kind, count and print files (deduplicated by file name,
    preferring upstream's curated Print-Files copy)."""
    files: dict[str, list[dict]] = {}
    for f in index["print_files"]:
        m = f["match"]
        if not m:
            continue
        for pid in [m["part"], *m.get("also", [])]:
            files.setdefault(pid, []).append(f)
    parts = {}
    for pid in sorted(set(instances.values())):
        p = index["parts"][pid]
        best: dict[str, dict] = {}
        for f in files.get(pid, []):
            name = f["path"].rsplit("/", 1)[-1]
            cur = best.get(name)
            if cur is None or (f["path"].startswith("Print-Files/") and not cur["path"].startswith("Print-Files/")):
                best[name] = f
        out = []
        for name, f in sorted(best.items(), key=lambda kv: (kv[1]["format"] in ("step", "stp"), kv[0])):
            row = {"name": name, "url": f["url"], "format": f["format"], "exact": f["match"]["quality"] == "exact"}
            for k in ("category", "quantity"):
                if k in f:
                    row[k] = f[k]
            out.append(row)
        parts[pid] = {"name": p["name"], "kind": p["kind"], "count": p["count"], "files": out}
    return {
        "variant": index["variant"],
        "title": index["title"],
        "source": index["source"],
        "instances": dict(sorted(instances.items())),
        "parts": parts,
        **({"belt": belt} if belt else {}),
    }


def _bbox_pts(shape):
    from .geom import bbox

    return bbox(shape)


def export(variant: str, cfg: dict, index: dict, root: Path, log=print) -> Path:
    web = cfg["web"]
    frame = np.eye(4)
    frame[:3] = np.array(web["frame"])
    asm = load(root / cfg["repo"] / cfg["assembly"])
    pairs = match_instances(index, asm)
    exclude = web.get("exclude", [])
    merge = [re.compile(p, re.I) for p in web.get("merge", [])]
    groups_cfg: dict[str, list[str]] = web.get("groups", {})
    default_group = web.get("default_group", "housing")

    def group_of(key: str, path: list[str]) -> str:
        for g, keys in groups_cfg.items():
            if key in keys or any(k in path for k in keys):
                return g
        return default_group

    glb = GLB()
    palette = Palette(glb)
    appearance = load_appearance(root / web["appearance"] if web.get("appearance") else None)
    part_mesh: dict[str, int] = {}
    group_children: dict[str, list[int]] = {}
    merged: dict[str, list[tuple[dict, object]]] = {}
    tris = 0

    exported: dict[str, str] = {}  # instance id -> part id
    for row, inst in sorted(pairs, key=lambda p: p[0]["id"]):
        if any(e in row["path"] for e in exclude):
            continue
        exported[row["id"]] = row["part"]
        m = next((m for m in merge for seg in row["path"] if m.search(seg)), None)
        if m:
            unit = next(seg for seg in row["path"] if m.search(seg))
            merged.setdefault(unit, []).append((row, inst))
            continue
        pid = row["part"]
        if pid not in part_mesh:
            part = index["parts"][pid]
            shape, colors = inst.proto.shape, asm.face_colors(inst)
            if pid in web.get("uncap", {}):
                shape, colors = uncap(shape, colors, web["uncap"][pid], part, pid, log)
            p, n, i, fot = tessellate_demo(shape)
            tris += len(i) // 3
            colors = face_colors_with_appearance(colors, pid, part, appearance, log)
            tri_colors = [colors[f] for f in fot]
            t3 = p[i.reshape(-1, 3)]
            area = np.linalg.norm(np.cross(t3[:, 1] - t3[:, 0], t3[:, 2] - t3[:, 0]), axis=1)
            themed = appearance.get(pid, {}).get("theme", True)
            mat = palette.triangles(part["kind"], tri_colors, area, themed)
            part_mesh[pid] = glb.mesh(pid, MeshData(p, n, i, mat))
        node = glb.node(Node(row["id"], frame @ np.vstack([inst.world[:3], [0, 0, 0, 1]]), part_mesh[pid]))
        group_children.setdefault(group_of(pid, row["path"]), []).append(node)

    # Merged rigid units: one mesh in assembly coordinates, named after the unit.
    for unit, members in merged.items():
        p, n, i, mat = assembly_mesh(
            [(inst.proto.shape, inst.world, asm.face_colors(inst)) for _, inst in members], palette, "electronics"
        )
        tris += len(i) // 3
        name = re.sub(r"[^a-z0-9]+", "-", re.sub(r"\s+v\d+$", "", unit).lower()).strip("-")
        mesh = glb.mesh(name, MeshData(p, n, i, mat))
        group_children.setdefault(group_of(name, members[0][0]["path"]), []).append(glb.node(Node(name, frame, mesh)))

    # Parts the index derived or added (nuts, extra bolts, …): geometry from their spec.
    belt = None
    for row in index["instances"]:
        part = index["parts"][row["part"]]
        if "shape" not in part or any(e in row["path"] for e in exclude):
            continue
        pid = row["part"]
        exported[row["id"]] = pid
        if part["shape"]["type"] == "belt":
            # Built live by the site from its route (so it can flex), not baked.
            belt = belt_data(row["id"], part["shape"], frame)
            continue
        if pid not in part_mesh:
            p, n, i, mat = shape_mesh(part["shape"], part["kind"], root / cfg["repo"], palette)
            tris += len(i) // 3
            part_mesh[pid] = glb.mesh(pid, MeshData(p, n, i, mat))
        world = np.vstack([np.array(row["transform"]), [0, 0, 0, 1]])
        node = glb.node(Node(row["id"], frame @ world, part_mesh[pid]))
        group_children.setdefault(group_of(pid, row["path"]), []).append(node)

    roots = [glb.node(Node(g, None, None, kids)) for g, kids in sorted(group_children.items())]
    out = root / web["out"]
    glb.write(out, roots)
    if web.get("catalog"):
        cat = root / web["catalog"]
        cat.parent.mkdir(parents=True, exist_ok=True)
        cat.write_text(json.dumps(catalog_data(index, exported, belt), indent=2, ensure_ascii=False) + "\n")
    log(f"{variant}: {len(part_mesh)} part meshes + {len(merged)} merged units, {tris} triangles "
        f"→ {out.relative_to(root)} ({out.stat().st_size / 1e6:.2f} MB)")
    return out
