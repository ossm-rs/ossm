"""Build the parts index for a hardware variant.

The index is generated data, committed so the website and future re-runs can
use it without re-reading STEP files:

- parts: each unique solid, with a stable id, a pose-independent geometric
  fingerprint, a short feature summary and the upstream print files it
  matches;
- instances: every placement of a part (assembly path + transform, mm);
- units: what moves as one during assembly, what each touches, and the
  directions it can slide free (for explode/build animations);
- print_files: every STL/3MF/STEP upstream ships, matched to a part by
  fingerprint (names differ between the assembly and the print files).

Ids are kept stable across upstream updates by matching against the
previous index (fingerprint first, then name), and the differences are
reported so changed parts can be re-checked.
"""

from __future__ import annotations

import fnmatch
import json
import re
import subprocess
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import quote

import numpy as np

from .meshio import Fingerprint, mesh_fingerprint, read_mesh
from .step import Instance, load

SCHEMA = 1
# Relative fingerprint distance under which two solids are the same part.
SAME = 0.02


def brep_fingerprint(shape) -> Fingerprint:
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps

    g = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, g)
    s = GProp_GProps()
    BRepGProp.SurfaceProperties_s(shape, s)
    return Fingerprint(g.Mass(), s.Mass(), tuple(sorted(float(m) for m in g.PrincipalProperties().Moments())))


def slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")


_VERSION = re.compile(r"\s+v\d+$", re.I)
_TIMESTAMP = re.compile(r"^\d{4}_\d{2}_\d{2}_")
_FASTENER = re.compile(r"Socket Head Cap Screw|\bscrew\b|\bbolt\b|dowel|SHCS", re.I)


def base_name(inst: Instance) -> str:
    """Human name for a placed solid, without Fusion's version/occurrence noise."""
    name = inst.proto.name
    if name in ("SOLID", "COMPOUND", "") and len(inst.path) >= 2:
        name = inst.path[-2]
    name = re.sub(r":\d+$", "", name)
    name = _VERSION.sub("", name)
    mirrored = "(Mirror)" in name
    name = re.sub(r"Broached Hexagon Socket Head Cap Screw ANSI B18\.3\.1M - (M\d+)x[\d.]+ x (\d+).*", r"SHCS \1x\2", name)
    name = re.sub(r"\s*\(Mirror\)", "", name)
    return f"{name.strip()} (mirrored)" if mirrored else name.strip()


def clean_path(path: tuple[str, ...]) -> list[str]:
    return [_VERSION.sub("", re.sub(r":\d+$", "", p)).strip() for p in path]


@dataclass
class PartRec:
    id: str
    name: str
    fp: Fingerprint
    shape: object
    members: list[Instance]
    group: str  # enclosing component, used to name parts upstream left as timestamps


def _git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, check=True).stdout.strip()


def _repo_url(root: Path, repo_rel: str) -> str:
    try:
        url = subprocess.run(
            ["git", "-C", str(root), "config", "-f", ".gitmodules", "--get", f"submodule.{repo_rel}.url"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    except subprocess.CalledProcessError:
        url = _git(root / repo_rel, "remote", "get-url", "origin")
    return re.sub(r"\.git$", "", url)


def _r(x: float, nd: int) -> float:
    v = round(float(x), nd)
    return 0.0 if v == 0 else v


def build(variant: str, cfg: dict, root: Path, previous: dict | None, log=print) -> dict:
    from .contacts import candidate_dirs, dedupe, find_contacts, free_directions, units
    from .geom import analyse, axis_label

    repo = root / cfg["repo"]
    sha = _git(repo, "rev-parse", "HEAD")
    url = _repo_url(root, cfg["repo"])
    asm = load(repo / cfg["assembly"])
    leaves, dropped = dedupe(asm.leaf_instances())
    log(f"{variant}: {len(leaves)} solids ({len(dropped)} duplicates dropped)")

    # --- parts: group placements by identical geometry + name ------------------
    fps: dict[str, Fingerprint] = {}
    boxes: dict[str, np.ndarray] = {}
    groups: dict[tuple[str, str], list[Instance]] = defaultdict(list)
    for inst in leaves:
        if inst.proto.entry not in fps:
            fps[inst.proto.entry] = brep_fingerprint(inst.proto.shape)
            boxes[inst.proto.entry] = np.concatenate(_bbox(inst.proto.shape))
        groups[(base_name(inst), inst.proto.entry)].append(inst)
    parts: list[PartRec] = []
    for (name, entry), members in groups.items():
        fp = fps[entry]
        # One part exported as several prototypes (the Light variant does this). The
        # local bounds must agree too: a mirrored copy has the same fingerprint but
        # its geometry is flipped in its own frame, so its transforms don't fit ours.
        same = next(
            (p for p in parts if p.name == name and p.fp.distance(fp) < 1e-4
             and np.allclose(boxes[p.members[0].proto.entry], boxes[entry], atol=0.01)),
            None,
        )
        if same:
            same.members += members
            continue
        path = clean_path(members[0].path)
        own_body = members[0].proto.name in ("SOLID", "COMPOUND", "")
        group = path[-3] if own_body and len(path) >= 3 else path[-2] if len(path) >= 2 else ""
        parts.append(PartRec("", name, fp, members[0].proto.shape, members, group))

    # Upstream names some solids by timestamp: name them after their component.
    for group, recs in _by(lambda p: p.group, [p for p in parts if _TIMESTAMP.match(p.name)]).items():
        for k, p in enumerate(sorted(recs, key=lambda p: -p.fp.volume)):
            p.name = f"{group} part {k + 1}"
    # Bare sizes ("16mm") only make sense with their component ("Dowels 16mm").
    for p in parts:
        if re.fullmatch(r"[\d.]+\s*mm", p.name) and p.group:
            p.name = f"{p.group} {p.name}"
    # Different parts sharing a name: qualify all but the largest by component.
    for name, recs in _by(lambda p: p.name.lower(), parts).items():
        for p in sorted(recs, key=lambda p: -p.fp.volume)[1:]:
            if p.group:
                p.name = f"{p.group} {p.name}"

    _assign_ids(parts, previous)

    # --- instances ------------------------------------------------------------
    inst_rows = []
    for p in sorted(parts, key=lambda p: p.id):
        ordered = sorted(p.members, key=lambda i: (clean_path(i.path), tuple(np.round(i.world[:3, 3], 2))))
        for k, inst in enumerate(ordered):
            inst_rows.append({
                "id": f"{p.id}#{k + 1}",
                "part": p.id,
                "path": clean_path(inst.path)[1:-1],
                "transform": _rows(inst.world),
            })
    _keep_instance_ids(inst_rows, previous)

    # --- units, contacts, free directions -------------------------------------
    us, _ = units(asm, cfg.get("rigid", []))
    contacts = find_contacts(us)
    touches = defaultdict(set)
    for c in contacts:
        if c.touching_kinds:
            touches[c.a].add(c.b)
    # units() builds its own Instance objects; map them to ours by part + position.
    lookup = {}
    for r in inst_rows:
        t = np.array(r["transform"])[:, 3]
        lookup.setdefault(r["part"], []).append((t, r["id"]))
    part_of_proto = {}
    for p in parts:
        for m in p.members:
            part_of_proto[m.proto.entry] = p.id

    def find_inst(m: Instance) -> str:
        cands = lookup[part_of_proto[m.proto.entry]]
        t = m.world[:3, 3]
        return min(cands, key=lambda c: np.linalg.norm(c[0] - t))[1]

    unit_rows = []
    everyone = set(range(len(us)))
    for i, u in enumerate(us):
        members = sorted(find_inst(m) for m in u.members)
        free = free_directions(i, everyone - {i}, contacts, us, candidate_dirs(u))
        unit_rows.append({
            "id": members[0] if len(members) == 1 else slug(clean_path(u.path)[-1]),
            "members": members,
            "touches": [],
            "free": [[_r(x, 3) for x in d] for d in free],
        })
    for i, row in enumerate(unit_rows):
        row["touches"] = sorted(unit_rows[j]["id"] for j in touches[i])

    # --- per-part summary -----------------------------------------------------
    # Sub-assemblies whose parts are electronics (names alone mislead: "PCB Housing" is printed).
    electronics = [re.compile(x) for x in cfg.get("electronics", [])]
    part_rows = {}
    for p in sorted(parts, key=lambda p: p.id):
        lo, hi = _bbox(p.shape)
        row = {
            "name": p.name,
            "upstream": sorted({m.proto.name for m in p.members}),
            "count": len(p.members),
            "size": [_r(x, 1) for x in hi - lo],
            "fingerprint": p.fp.to_json(),
            "kind": "fastener" if _FASTENER.search(p.name) else "electronics" if _in_any(p, electronics) else "",
            "print_files": [],
        }
        if not row["kind"] and p.fp.volume > 50:
            rep = analyse(p.name, p.shape)
            feats = defaultdict(int)
            for f in rep.axes:
                if f.kind == "hole":
                    feats[f"{axis_label(f.direction)}: {f.describe()}"] += 1
            row["holes"] = [f"{n}× {d}" if n > 1 else d for d, n in sorted(feats.items())]
        part_rows[p.id] = row

    # --- print files ------------------------------------------------------------
    files = _print_files(repo, cfg["print_files"])
    log(f"{variant}: fingerprinting {len(files)} print files")
    file_rows = []
    for rel in files:
        fp = _file_fingerprint(repo / rel)
        if fp is None:
            continue
        match = _match_file(rel, fp, parts)
        row = {
            "path": rel,
            "url": f"{url}/blob/{sha}/{quote(rel)}",
            "format": Path(rel).suffix.lower().lstrip("."),
            "fingerprint": fp.to_json(),
            "match": match,
        }
        q = re.search(r"\(x(\d+)\)", rel)
        if q:
            row["quantity"] = int(q.group(1))
        cat = re.search(r"/(Strength|Decorative) Parts/", rel)
        if cat:
            row["category"] = cat.group(1).lower()
        file_rows.append(row)
        if match:
            for pid in [match["part"], *match.get("also", [])]:
                part_rows[pid]["print_files"].append(rel)
    for pid, row in part_rows.items():
        if not row["kind"]:
            # Printed if upstream ships a print file for it; otherwise bought-in hardware.
            row["kind"] = "printed" if row["print_files"] else "hardware"

    _add_derived(parts, part_rows, inst_rows, cfg, repo, log)

    index = {
        "schema": SCHEMA,
        "variant": variant,
        "title": cfg.get("title", variant),
        "source": {"repo": url, "commit": sha, "assembly": cfg["assembly"]},
        "units": "mm",
        "duplicates_dropped": len(dropped),
        "parts": part_rows,
        "instances": inst_rows,
        "assembly_units": unit_rows,
        "print_files": file_rows,
    }
    return index


def _rows(m: np.ndarray) -> list[list[float]]:
    return [[_r(v, 6 if c < 3 else 3) for c, v in enumerate(row)] for row in m[:3]]


def _add_derived(parts: list[PartRec], part_rows: dict, inst_rows: list[dict], cfg: dict, repo: Path, log) -> None:
    """Parts upstream doesn't model: nuts in printed pockets, and authored additions.

    They become ordinary parts and instances (with a `shape` spec to rebuild their
    geometry and a `source` saying where they came from), so the BOM, catalog and
    change report treat them like everything else.
    """
    from .derived import belt_path, frame_from_axis, hex_nut, nut_pockets, shcs, Pulley
    from .geom import analyse
    from .meshio import mesh_fingerprint

    row_of = {}
    by_part = _by(lambda r: r["part"], inst_rows)
    for p in parts:
        for r in by_part.get(p.id, []):
            row_of[r["id"]] = (p, np.vstack([np.array(r["transform"]), [0, 0, 0, 1]]))

    def add_part(pid: str, name: str, kind: str, fp: Fingerprint, shape: dict, source: str, size) -> None:
        part_rows[pid] = {
            "name": name, "upstream": [], "count": 0, "size": [_r(x, 1) for x in size],
            "fingerprint": fp.to_json(), "kind": kind, "print_files": [], "source": source, "shape": shape,
        }

    def add_instance(pid: str, path: list[str], world: np.ndarray, **extra) -> str:
        part_rows[pid]["count"] += 1
        iid = f"{pid}#{part_rows[pid]['count']}"
        inst_rows.append({"id": iid, "part": pid, "path": path, "transform": _rows(world), **extra})
        return iid

    # Nuts: one per pocket, in every printed part's placements.
    printed = [(rid, p, w) for rid, (p, w) in sorted(row_of.items()) if part_rows[p.id]["kind"] == "printed"]
    pockets = {p.id: nut_pockets(p.shape) for p in {p.id: p for _, p, _ in printed}.values()}
    for rid, p, w in printed:
        for pk in pockets[p.id]:
            pid = f"nut-m{pk.size}"
            if pid not in part_rows:
                nut = hex_nut(pk.size)
                lo, hi = _bbox(nut)
                add_part(pid, f"M{pk.size} hex nut", "fastener", brep_fingerprint(nut),
                         {"type": "hex-nut", "size": pk.size}, "derived: nut pocket", hi - lo)
            host = next(r for r in inst_rows if r["id"] == rid)
            add_instance(pid, host["path"] + [part_rows[p.id]["name"]], w @ pk.frame, host=f"{rid}/{pk.hole}")
    log(f"  derived {sum(len(v) for v in pockets.values())} nut pockets")

    # Hole axes in the assembly, for snapping authored bolts onto them.
    holes = []
    for rid, p, w in printed:
        for ax in analyse("", p.shape).axes:
            if any(s.concave for s in ax.segments):
                holes.append((w[:3, :3] @ ax.origin + w[:3, 3], w[:3, :3] @ ax.direction))

    for pid, spec in cfg.get("additions", {}).items():
        shape = dict(spec["shape"])
        kind, name = spec["kind"], spec["name"]
        if shape["type"] == "belt":
            path3d, plane, route, length, lead_counts = _belt(shape, inst_rows, row_of, Pulley, belt_path)
            shape.update({
                "path": [[_r(x, 3) for x in pt] for pt in path3d], "plane": _rows(plane),
                "route": route, "length": _r(length, 2), "lead_counts": lead_counts,
            })
            from .derived import belt_mesh
            v, t = belt_mesh((path3d - plane[:3, 3]) @ plane[:3, :2], plane, shape["width"], shape["thickness"])
            add_part(pid, name, kind, mesh_fingerprint(v[t]), shape, "authored", np.ptp(v, axis=0))
            add_instance(pid, ["belt"], np.eye(4))
            continue
        if shape["type"] == "hex-nut":
            solid = hex_nut(shape["size"])
        elif shape["type"] == "shcs":
            solid = shcs(shape["d"], shape["length"])
        elif shape["type"] == "step":
            solid = _step_compound(repo / shape["file"])
        else:
            raise ValueError(f"unknown addition shape {shape['type']!r}")
        lo, hi = _bbox(solid)
        if pid not in part_rows:  # may add placements to a derived part (a nut with no pocket)
            add_part(pid, name, kind, brep_fingerprint(solid), shape, "authored", hi - lo)
        for pl in spec["placements"]:
            if "transform" in pl:
                world = np.vstack([np.array(pl["transform"], float), [0, 0, 0, 1]])
            else:
                head, d = np.array(pl["head"], float), np.array(pl["dir"], float)
                d /= np.linalg.norm(d)
                # Snap onto the nearest hole axis: upstream geometry beats hand placement.
                best = min(holes, key=lambda h: np.linalg.norm(np.cross(head - h[0], h[1])) if abs(np.dot(h[1], d)) > 0.98 else 1e9)
                if abs(np.dot(best[1], d)) > 0.98 and np.linalg.norm(np.cross(head - best[0], best[1])) < 6:
                    axis = best[1] * np.sign(np.dot(best[1], d))
                    head = best[0] + axis * np.dot(head - best[0], axis)
                    d = axis
                world = frame_from_axis(head, d)
            add_instance(pid, pl.get("path", []), world, source="authored")


def _step_compound(path: Path):
    from OCP.BRep import BRep_Builder
    from OCP.TopoDS import TopoDS_Compound

    from .geom import transformed

    b = BRep_Builder()
    comp = TopoDS_Compound()
    b.MakeCompound(comp)
    for inst in load(path).leaf_instances():
        b.Add(comp, transformed(inst.proto.shape, inst.world))
    return comp


def _belt(spec: dict, inst_rows: list[dict], row_of: dict, Pulley, belt_path):
    """Belt centreline: end → idler stack → pulley → idler stack → end, in the idlers' plane."""
    def centre(rid):
        p, w = row_of[rid]
        lo, hi = _bbox(p.shape)
        return w[:3, :3] @ ((lo + hi) / 2) + w[:3, 3]

    idlers = [r["id"] for r in inst_rows if r["part"] == spec["idlers"]]
    p0, w0 = row_of[idlers[0]]
    lo, hi = _bbox(p0.shape)
    size = hi - lo
    axis = w0[:3, :3] @ np.eye(3)[int(np.argmin(size))]  # bearings are flat along their axis
    r_idler = float(np.sort(size)[-1]) / 2
    # Stacks: bearings sharing an axis line.
    stacks: list[list[tuple[str, np.ndarray]]] = []
    for rid in idlers:
        c = centre(rid)
        for st in stacks:
            if np.linalg.norm(np.cross(c - st[0][1], axis)) < 1:
                st.append((rid, c))
                break
        else:
            stacks.append([(rid, c)])
    stack_c = [np.mean([c for _, c in st], axis=0) for st in stacks]
    stack_ids = [[rid for rid, _ in st] for st in stacks]
    pulley_id = next(r["id"] for r in inst_rows if r["part"] == spec["pulley"])
    pulley_c = centre(pulley_id)
    ends = [centre(e) for e in spec["ends"]]

    # Plane: origin on the idler stacks' mid-plane, x along the run (end to end), z = axis.
    origin = np.mean(stack_c, axis=0)
    x = ends[1] - ends[0]
    x -= axis * np.dot(x, axis)
    x /= np.linalg.norm(x)
    plane = np.eye(4)
    plane[:3, 0], plane[:3, 1], plane[:3, 2], plane[:3, 3] = x, np.cross(axis, x), axis, origin

    def to2(p):
        q = p - origin
        return np.array([np.dot(q, plane[:3, 0]), np.dot(q, plane[:3, 1])])

    t = spec["thickness"]
    pc = to2(pulley_c)
    order = sorted(range(len(stack_c)), key=lambda k: np.linalg.norm(to2(stack_c[k]) - to2(ends[0])))
    st2 = [to2(stack_c[k]) for k in order]
    stack_ids = [stack_ids[k] for k in order]
    # The belt runs over the idlers, on the side away from the pulley, then under the pulley.
    away = np.sign(np.mean([q[1] for q in st2]) - pc[1]) or 1.0
    level = st2[0][1] + away * (r_idler + t / 2)
    a, b = st2[0] - pc, st2[1] - pc
    pulley_ccw = a[0] * b[1] - a[1] * b[0] < 0
    # Each end: authored lead through its clamp (tip first), or a bare point at the
    # clamp level with the run. `carriers` names the node(s) each route point moves with.
    leads = spec.get("leads", {})
    def end(which: str, rid: str, at_u: float, reverse: bool):
        lead = leads.get(which)
        if not lead:
            return [np.array([at_u, level])], [[rid]]
        els = [Pulley(np.array(e["centre"], float), e["radius"], e["ccw"]) if isinstance(e, dict)
               else np.array(e, float) for e in lead["path"]]
        if reverse:  # authored tip first; the run reaches the rear end last
            els = [Pulley(e.centre, e.radius, not e.ccw) if isinstance(e, Pulley) else e for e in reversed(els)]
        return els, [[lead["node"]]] * len(els)
    front, front_by = end("front", spec["ends"][0], to2(ends[0])[0], False)
    rear, rear_by = end("rear", spec["ends"][1], to2(ends[1])[0], True)
    pts = [
        *front,
        Pulley(st2[0], r_idler + t / 2, not pulley_ccw),
        Pulley(pc, spec["pitch_diameter"] / 2, pulley_ccw),
        Pulley(st2[1], r_idler + t / 2, not pulley_ccw),
        *rear,
    ]
    carriers = [*front_by, stack_ids[0], [pulley_id], stack_ids[1], *rear_by]
    from .derived import with_tails

    route_len = float(np.sum(np.linalg.norm(np.diff(belt_path(pts), axis=0), axis=1)))
    path2 = with_tails(belt_path(pts), spec.get("length"))
    path3 = origin + np.outer(path2[:, 0], plane[:3, 0]) + np.outer(path2[:, 1], plane[:3, 1])
    # The same route as data, for rebuilding the belt live: each anchor and wrap
    # names the node(s) it moves with.
    r2 = lambda q: [_r(float(x), 3) for x in q]
    route = []
    for el, nodes in zip(pts, carriers):
        if isinstance(el, Pulley):
            route.append({"wrap": nodes, "centre": r2(el.centre), "radius": _r(el.radius, 3), "ccw": bool(el.ccw)})
        else:
            route.append({"anchor": nodes[0], "at": r2(el)})
    # How many route elements each end's lead has (0: a bare end), so the site
    # knows where the belt leaves a clamp.
    lead_counts = {"front": len(front) if leads.get("front") else 0, "rear": len(rear) if leads.get("rear") else 0}
    length = float(spec.get("length") or route_len)
    if length < route_len:
        print(f"  belt: {length} mm is shorter than its route ({route_len:.1f} mm)")
    return path3, plane, route, length, lead_counts


def _by(key, items):
    out = defaultdict(list)
    for x in items:
        out[key(x)].append(x)
    return out


def _bbox(shape):
    from .geom import bbox

    return bbox(shape)


def _in_any(p: PartRec, patterns: list[re.Pattern]) -> bool:
    return any(rx.search(seg) for rx in patterns for seg in clean_path(p.members[0].path[:-1]))


def _assign_ids(parts: list[PartRec], previous: dict | None) -> None:
    """Reuse ids from the previous index (same name + geometry, then geometry, then name)."""
    taken: set[str] = set()
    if previous:
        prev = [(pid, row["name"], Fingerprint.from_json(row["fingerprint"])) for pid, row in previous["parts"].items()]
        for rule in ("both", "geometry", "name"):
            for p in parts:
                if p.id:
                    continue
                for pid, name, fp in prev:
                    if pid in taken:
                        continue
                    geo = p.fp.distance(fp) < SAME
                    ok = {"both": geo and name == p.name, "geometry": geo, "name": name == p.name}[rule]
                    if ok:
                        p.id = pid
                        taken.add(pid)
                        break
    for p in sorted(parts, key=lambda p: (p.name, -p.fp.volume)):
        if p.id:
            continue
        base = slug(p.name) or "part"
        pid, n = base, 2
        while pid in taken:
            pid, n = f"{base}-{n}", n + 1
        p.id = pid
        taken.add(pid)


def _keep_instance_ids(rows: list[dict], previous: dict | None) -> None:
    """Instance ids follow the previous index where a part's placement is (nearly) unchanged."""
    if not previous:
        return
    prev = _by(lambda r: r["part"], previous["instances"])
    for part, mine in _by(lambda r: r["part"], rows).items():
        old = list(prev.get(part, []))
        for r in mine:
            if not old:
                break
            t = np.array(r["transform"])[:, 3]
            j = min(range(len(old)), key=lambda k: np.linalg.norm(np.array(old[k]["transform"])[:, 3] - t))
            if np.linalg.norm(np.array(old[j]["transform"])[:, 3] - t) < 5.0:
                r["id"] = old[j]["id"]
            old.pop(j)


def _print_files(repo: Path, spec: dict) -> list[str]:
    formats = {"." + f.lower() for f in spec["formats"]}
    out = []
    for d in spec["dirs"]:
        for p in (repo / d).rglob("*"):
            rel = p.relative_to(repo).as_posix()
            if p.is_file() and p.suffix.lower() in formats and not any(fnmatch.fnmatch(rel, x) for x in spec.get("exclude", [])):
                out.append(rel)
    return sorted(out)


def _file_fingerprint(path: Path) -> Fingerprint | None:
    if path.suffix.lower() in (".step", ".stp"):
        asm = load(path)
        shapes = [i.proto.shape for i in asm.leaf_instances()]
        if len(shapes) != 1:
            return None  # multi-body files are reference assemblies, not single parts
        return brep_fingerprint(shapes[0])
    tris = read_mesh(path)
    return mesh_fingerprint(tris) if len(tris) else None


def _tokens(text: str) -> set[str]:
    return {t for t in re.findall(r"[a-z]{3,}", re.sub(r"([a-z])([A-Z])", r"\1 \2", text).lower())} - {"mount", "single", "color", "branding", "multicolor", "step", "stl"}


def _match_file(rel: str, fp: Fingerprint, parts: list[PartRec]) -> dict | None:
    """Pair a print file with a part.

    exact:   same geometry (within SAME);
    variant: same part, different revision/option: either volume and inertia
             within 3% (e.g. branding embossed or not; area is ignored), or
             within 20% and sharing a name word.
    Mirrored parts (left/right) have identical fingerprints; all are listed.
    """
    multi = rel.lower().endswith(".3mf")  # colour regions add internal area
    ranked = sorted(parts, key=lambda p: fp.distance(p.fp, use_area=not multi))
    best = ranked[0]
    d = fp.distance(best.fp, use_area=not multi)
    quality = None
    if d < SAME:
        quality = "exact"
    else:
        best = min(parts, key=lambda p: fp.distance(p.fp, use_area=False))
        d = fp.distance(best.fp, use_area=False)
        if d < 0.03:
            quality = "variant"
        else:
            words = _tokens(Path(rel).stem)
            named = [
                p for p in parts
                if words & _tokens(p.name) or any(w in re.sub(r"[^a-z]", "", p.name.lower()) for w in words)
            ]
            named = [p for p in named if abs(p.fp.volume - fp.volume) / max(p.fp.volume, fp.volume) < 0.2]
            if named:
                best = min(named, key=lambda p: fp.distance(p.fp, use_area=False))
                d = fp.distance(best.fp, use_area=False)
                quality = "variant"
    if not quality:
        return None
    out = {"part": best.id, "quality": quality, "distance": _r(d, 4)}
    twins = sorted(p.id for p in parts if p is not best and p.fp.distance(best.fp) < 1e-4)
    if twins:
        out["also"] = twins
    return out


def changes(old: dict, new: dict) -> list[str]:
    """What an upstream update changed, in terms of our ids."""
    out = []
    op, np_ = old["parts"], new["parts"]
    for pid in sorted(set(op) - set(np_)):
        out.append(f"removed part   {pid}")
    for pid in sorted(set(np_) - set(op)):
        out.append(f"added part     {pid}  ({np_[pid]['name']})")
    for pid in sorted(set(op) & set(np_)):
        a, b = Fingerprint.from_json(op[pid]["fingerprint"]), Fingerprint.from_json(np_[pid]["fingerprint"])
        if a.distance(b) >= 1e-3:
            out.append(f"geometry       {pid}  (Δ {a.distance(b):.1%})")
        if op[pid]["count"] != np_[pid]["count"]:
            out.append(f"quantity       {pid}  {op[pid]['count']} → {np_[pid]['count']}")
        if op[pid]["kind"] != np_[pid]["kind"]:
            out.append(f"kind           {pid}  {op[pid]['kind']} → {np_[pid]['kind']}")
    oi = {r["id"]: r for r in old["instances"]}
    for r in new["instances"]:
        o = oi.get(r["id"])
        if o and not np.allclose(np.array(o["transform"]), np.array(r["transform"]), atol=0.01):
            out.append(f"moved          {r['id']}")
    of = {f["path"]: f.get("match") for f in old["print_files"]}
    for f in new["print_files"]:
        if f["path"] not in of:
            out.append(f"new file       {f['path']}")
        elif (of[f["path"]] or {}).get("part") != (f.get("match") or {}).get("part"):
            out.append(f"file rematched {f['path']}")
    for p in sorted(set(of) - {f["path"] for f in new["print_files"]}):
        out.append(f"file removed   {p}")
    return out


def write(index: dict, path: Path) -> None:
    """Pretty JSON, but numeric arrays kept on one line so diffs stay readable."""
    text = json.dumps(index, indent=2, ensure_ascii=False)
    text = re.sub(r"\[\s+([-\d.e,\s]+?)\s+\]", lambda m: "[" + re.sub(r"\s+", " ", m.group(1)).replace(" ,", ",") + "]", text)
    text = re.sub(r"\[\s+(\[[^\[\]]*\](?:,\s+\[[^\[\]]*\])*)\s+\]", lambda m: "[" + re.sub(r",\s+", ", ", m.group(1)) + "]", text)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text + "\n")
