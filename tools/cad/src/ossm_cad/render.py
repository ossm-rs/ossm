"""Headless contact-sheet renderer: tessellate the B-rep, rasterise with PIL.

No OpenGL: triangles are painter-sorted and filled with PIL, which is fast
enough for single parts and small assemblies and works on any machine. A
parallel face-id buffer tells us which features are actually visible, so
labels are only placed on what the viewer can see.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from OCP.BRep import BRep_Tool
from OCP.BRepMesh import BRepMesh_IncrementalMesh
from OCP.TopAbs import TopAbs_REVERSED
from OCP.TopLoc import TopLoc_Location
from OCP.TopoDS import TopoDS_Shape
from PIL import Image, ImageDraw, ImageFont

from .geom import faces


@dataclass
class Mesh:
    verts: np.ndarray  # (N, 3)
    tris: np.ndarray  # (M, 3) vertex indices
    face_of_tri: np.ndarray  # (M,) B-rep face index per triangle
    group_of_tri: np.ndarray  # (M,) caller-defined group (e.g. instance index)


def tessellate(shape: TopoDS_Shape, deflection: float = 0.05) -> Mesh:
    BRepMesh_IncrementalMesh(shape, deflection, False, 0.3, True)
    verts, tris, fot = [], [], []
    base = 0
    for fi, face in enumerate(faces(shape)):
        loc = TopLoc_Location()
        tri = BRep_Tool.Triangulation_s(face, loc)
        if tri is None:
            continue
        trsf = loc.Transformation()
        n = tri.NbNodes()
        pts = np.empty((n, 3))
        for i in range(n):
            p = tri.Node(i + 1).Transformed(trsf)
            pts[i] = (p.X(), p.Y(), p.Z())
        idx = np.array([tri.Triangle(i + 1).Get() for i in range(tri.NbTriangles())], dtype=np.int64) - 1
        if face.Orientation() == TopAbs_REVERSED:
            idx = idx[:, [0, 2, 1]]
        verts.append(pts)
        tris.append(idx + base)
        fot.append(np.full(len(idx), fi))
        base += n
    t = np.concatenate(tris)
    return Mesh(np.concatenate(verts), t, np.concatenate(fot), np.zeros(len(t), dtype=np.int64))


def merge(meshes: list[Mesh]) -> Mesh:
    verts, tris, fot, got = [], [], [], []
    base = 0
    for g, m in enumerate(meshes):
        verts.append(m.verts)
        tris.append(m.tris + base)
        fot.append(m.face_of_tri)
        got.append(np.full(len(m.tris), g))
        base += len(m.verts)
    return Mesh(np.concatenate(verts), np.concatenate(tris), np.concatenate(fot), np.concatenate(got))


# View name -> (direction the camera looks along, screen-up vector), model axes.
VIEWS: dict[str, tuple[tuple[float, float, float], tuple[float, float, float]]] = {
    "iso": ((-1, -1, -1), (0, 0, 1)),
    "iso-rear": ((1, 1, -1), (0, 0, 1)),
    "top (-Z)": ((0, 0, -1), (0, 1, 0)),
    "bottom (+Z)": ((0, 0, 1), (0, -1, 0)),
    "front (+Y)": ((0, 1, 0), (0, 0, 1)),
    "back (-Y)": ((0, -1, 0), (0, 0, 1)),
    "right (-X)": ((-1, 0, 0), (0, 0, 1)),
    "left (+X)": ((1, 0, 0), (0, 0, 1)),
}


def _basis(look, up):
    f = np.asarray(look, float)
    f /= np.linalg.norm(f)
    u = np.asarray(up, float)
    r = np.cross(f, u)
    if np.linalg.norm(r) < 1e-6:
        r = np.cross(f, [1, 0, 0])
    r /= np.linalg.norm(r)
    u = np.cross(r, f)
    return r, u, f


@dataclass
class Label:
    text: str
    face_ids: set[int]
    group: int | None = None  # restrict to one group (instance) when set
    # For holes/shafts: axis line. When viewed end-on the wall is edge-on, so the
    # label is anchored at the projected axis instead of on visible pixels.
    axis: tuple[np.ndarray, np.ndarray] | None = None  # (point, direction)


def _font(size: int):
    for name in ("DejaVuSans.ttf", "LiberationSans-Regular.ttf", "Arial.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            pass
    return ImageFont.load_default(size)


def render_view(
    mesh: Mesh,
    look,
    up,
    size: int,
    colors: np.ndarray,
    labels: list[Label],
    title: str,
    extent: float,
    centre: np.ndarray,
    ss: int = 2,
) -> Image.Image:
    """`colors`: (M, 3) per-triangle base RGB 0..1. `extent`: mm across the frame."""
    W = size * ss
    r, u, f = _basis(look, up)
    rel = mesh.verts - centre
    sx = rel @ r
    sy = rel @ u
    depth = rel @ f
    scale = W * 0.86 / extent
    px = W / 2 + sx * scale
    py = W / 2 - sy * scale

    tri = mesh.tris
    a, b, c = mesh.verts[tri[:, 0]], mesh.verts[tri[:, 1]], mesh.verts[tri[:, 2]]
    n = np.cross(b - a, c - a)
    nl = np.linalg.norm(n, axis=1, keepdims=True)
    n = np.divide(n, nl, out=np.zeros_like(n), where=nl > 0)
    facing = n @ f < 0
    # Headlight plus a key light from upper-left; keeps both flats and bores readable.
    key = -f * 0.6 + u * 0.6 - r * 0.5
    key /= np.linalg.norm(key)
    shade = 0.35 + 0.45 * np.clip(-(n @ f), 0, 1) + 0.3 * np.clip(n @ key, 0, 1)
    rgb = np.clip(colors * shade[:, None], 0, 1)

    order = np.argsort(-depth[tri].mean(axis=1))
    order = order[facing[order]]

    img = Image.new("RGB", (W, W), (250, 250, 248))
    ids = Image.new("I", (W, W), -1)
    d_img, d_ids = ImageDraw.Draw(img), ImageDraw.Draw(ids)
    tri_px = np.stack([px[tri], py[tri]], axis=-1)
    for t in order:
        pts = [tuple(p) for p in tri_px[t]]
        col = tuple(int(255 * x) for x in rgb[t])
        d_img.polygon(pts, fill=col, outline=col)
        d_ids.polygon(pts, fill=int(t))

    # Silhouette/feature edges: where the visible triangle's B-rep face or group changes.
    idarr = np.array(ids)
    key_arr = np.where(idarr >= 0, mesh.face_of_tri[np.maximum(idarr, 0)] * 1000 + mesh.group_of_tri[np.maximum(idarr, 0)], -1)
    edge = np.zeros_like(idarr, dtype=bool)
    edge[:, 1:] |= key_arr[:, 1:] != key_arr[:, :-1]
    edge[1:, :] |= key_arr[1:, :] != key_arr[:-1, :]
    arr = np.array(img)
    arr[edge] = (arr[edge] * 0.35).astype(np.uint8)
    img = Image.fromarray(arr)

    img = img.resize((size, size), Image.LANCZOS)
    draw = ImageDraw.Draw(img)
    font = _font(13)
    small = _font(11)

    # Labels: centroid of each feature's visible pixels.
    vis_face = np.where(idarr >= 0, mesh.face_of_tri[np.maximum(idarr, 0)], -1)
    vis_group = np.where(idarr >= 0, mesh.group_of_tri[np.maximum(idarr, 0)], -1)
    placed: list[tuple[float, float]] = []
    for lab in labels:
        if lab.axis is not None and abs(np.dot(lab.axis[1], f)) > 0.97:
            q = lab.axis[0] - centre
            x, y = (W / 2 + (q @ r) * scale) / ss, (W / 2 - (q @ u) * scale) / ss
        else:
            mask = np.isin(vis_face, list(lab.face_ids))
            if lab.group is not None:
                mask &= vis_group == lab.group
            if mask.sum() < 4 * ss * ss:
                continue
            ys, xs = np.nonzero(mask)
            # Pick the visible pixel nearest the centroid so the label sits on the feature.
            cx, cy = xs.mean(), ys.mean()
            k = np.argmin((xs - cx) ** 2 + (ys - cy) ** 2)
            x, y = xs[k] / ss, ys[k] / ss
        while any(abs(x - qx) < 22 and abs(y - qy) < 12 for qx, qy in placed):
            y += 12
        placed.append((x, y))
        tb = draw.textbbox((x, y), lab.text, font=small, anchor="mm")
        draw.rectangle((tb[0] - 2, tb[1] - 1, tb[2] + 2, tb[3] + 1), fill=(255, 255, 255), outline=(40, 40, 40))
        draw.text((x, y), lab.text, fill=(20, 20, 20), font=small, anchor="mm")

    # Axis triad (model axes as seen in this view) and a scale bar.
    ox, oy = 34, size - 34
    for vec, name, col in (((1, 0, 0), "X", (200, 40, 40)), ((0, 1, 0), "Y", (40, 150, 40)), ((0, 0, 1), "Z", (40, 70, 210))):
        v = np.asarray(vec, float)
        dx, dy = v @ r, -(v @ u)
        if abs(dx) + abs(dy) < 0.05:
            draw.text((ox - 4, oy - 6), "⊙" if v @ f < 0 else "⊗", fill=col, font=font)
            draw.text((ox + 6, oy - 16), name, fill=col, font=small)
            continue
        draw.line((ox, oy, ox + 22 * dx, oy + 22 * dy), fill=col, width=2)
        draw.text((ox + 28 * dx - 4, oy + 28 * dy - 7), name, fill=col, font=small)
    bar_mm = _nice(extent / 5)
    bar_px = bar_mm * scale / ss
    bx, by = size - 20 - bar_px, size - 24
    draw.line((bx, by, bx + bar_px, by), fill=(30, 30, 30), width=2)
    draw.text((bx + bar_px / 2, by - 4), f"{bar_mm:g} mm", fill=(30, 30, 30), font=small, anchor="mb")
    draw.text((10, 8), title, fill=(30, 30, 30), font=font)
    return img


def _nice(x: float) -> float:
    e = 10 ** np.floor(np.log10(x))
    for m in (1, 2, 5, 10):
        if m * e >= x:
            return float(m * e)
    return float(10 * e)


def contact_sheet(
    mesh: Mesh,
    colors: np.ndarray,
    labels: list[Label],
    header: str,
    views: list[str] = ("iso", "top (-Z)", "front (+Y)", "right (-X)"),
    size: int = 560,
) -> Image.Image:
    lo, hi = mesh.verts.min(axis=0), mesh.verts.max(axis=0)
    centre = (lo + hi) / 2
    extent = float(np.linalg.norm(hi - lo))
    cols = 2
    rows = (len(views) + 1) // 2
    head = 30
    sheet = Image.new("RGB", (cols * size, rows * size + head), (255, 255, 255))
    ImageDraw.Draw(sheet).text((10, 7), header, fill=(0, 0, 0), font=_font(15))
    for i, v in enumerate(views):
        look, up = VIEWS[v]
        tile = render_view(mesh, look, up, size, colors, labels, v, extent, centre)
        x, y = (i % cols) * size, head + (i // cols) * size
        sheet.paste(tile, (x, y))
        ImageDraw.Draw(sheet).rectangle((x, y, x + size - 1, y + size - 1), outline=(200, 200, 200))
    return sheet
