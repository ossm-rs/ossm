"""`just cad <command>`: inspect upstream STEP files.

Outputs are written for both people and Claude to read: plain-text reports on
stdout, and contact-sheet PNGs (multi-view renders with labelled features)
under --out.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

# Bought-in sub-assemblies that move as one piece.
DEFAULT_RIGID = ["OSSM PCB", "57AIM30", "Rail", "Bearing"]
TOOL_ROOT = Path(__file__).resolve().parents[2]
REPO_ROOT = TOOL_ROOT.parents[1]
DEFAULT_OUT = TOOL_ROOT / "out"
VARIANTS = TOOL_ROOT / "variants.json"


def _short(name: str) -> str:
    """Shorten Fusion's verbose fastener names: 'SHCS M3x0.5 x 12'."""
    name = re.sub(r"Broached Hexagon Socket Head Cap Screw ANSI B18\.3\.1M - ", "SHCS ", name)
    return re.sub(r" Steel Grade 2 Plain", "", name)


def _vec(v) -> str:
    return "(" + ", ".join(f"{x:.1f}" for x in v) + ")"


def cmd_tree(args) -> None:
    from .contacts import dedupe
    from .step import load

    asm = load(args.step)
    leaves = asm.leaf_instances()
    _, dropped = dedupe(leaves)

    def walk(proto, depth):
        counts = Counter(c.proto.entry for c in proto.children)
        seen = set()
        for c in proto.children:
            if c.proto.entry in seen:
                continue
            seen.add(c.proto.entry)
            n = counts[c.proto.entry]
            # Anonymous Fusion component bodies ("=>[0:1:1:43]" -> SOLID).
            name = f"{proto.name} (own body)" if c.proto.name in ("SOLID", "COMPOUND", "") else c.proto.name
            print("  " * depth + f"{'▸' if c.proto.is_assembly else '·'} {_short(name)}" + (f"  ×{n}" if n > 1 else ""))
            if c.proto.is_assembly and depth < args.depth:
                walk(c.proto, depth + 1)

    print(f"{asm.path.name}: {len(leaves)} placed solids, {len({l.proto.entry for l in leaves})} unique")
    if dropped:
        print(f"⚠ {len(dropped)} solids exactly duplicate another at the same position (ignored by analysis):")
        for path, n in Counter(d.path[1:3] for d in dropped).most_common(8):
            print(f"    {n:3} under {' / '.join(p.split(':')[0] for p in path)}")
    for root in asm.roots:
        print(f"▸ {root.name}")
        walk(root, 1)


def _parts_table(asm) -> dict[tuple, dict]:
    """Unique solids with counts, keyed by (short name, sorted bbox dims)."""
    from .contacts import dedupe
    from .geom import bbox

    keep, _ = dedupe(asm.leaf_instances())
    rows: dict[tuple, dict] = {}
    for inst in keep:
        lo, hi = bbox(inst.proto.shape)
        dims = tuple(sorted(float(x) for x in np.round(hi - lo, 1)))
        key = (_display(inst), dims)
        r = rows.setdefault(key, {"name": key[0], "dims": dims, "count": 0, "where": set()})
        r["count"] += 1
        r["where"].add(inst.path[1].split(":")[0] if len(inst.path) > 2 else "—")
    return rows


def cmd_diff(args) -> None:
    from .step import load

    a, b = load(args.a), load(args.b)
    ra, rb = _parts_table(a), _parts_table(b)
    names_a, names_b = defaultdict(list), defaultdict(list)
    for k in ra:
        names_a[k[0]].append(k)
    for k in rb:
        names_b[k[0]].append(k)

    def dims(d):
        return "×".join(f"{x:g}" for x in d)

    print(f"A = {a.path.name}\nB = {b.path.name}\n")
    same, recount, changed, only_a, only_b = [], [], [], [], []
    for name in sorted(set(names_a) | set(names_b)):
        ka, kb = names_a.get(name, []), names_b.get(name, [])
        if not kb:
            only_a += [ra[k] for k in ka]
        elif not ka:
            only_b += [rb[k] for k in kb]
        elif {k[1] for k in ka} == {k[1] for k in kb}:
            ca, cb = sum(ra[k]["count"] for k in ka), sum(rb[k]["count"] for k in kb)
            (same if ca == cb else recount).append((name, ca, cb))
        else:
            changed.append((name, [k[1] for k in ka], [k[1] for k in kb]))

    print(f"Identical in both ({len(same)}):")
    print("  " + ", ".join(n + (f" ×{c}" if c > 1 else "") for n, c, _ in same))
    if recount:
        print("\nSame part, different quantity:")
        for n, ca, cb in recount:
            print(f"  {n}: A ×{ca}, B ×{cb}")
    if changed:
        print("\nSame name, different geometry (bbox mm, sorted):")
        for n, da, db in changed:
            print(f"  {n}: A {', '.join(dims(d) for d in da)}  →  B {', '.join(dims(d) for d in db)}")
    for label, rows in (("Only in A", only_a), ("Only in B", only_b)):
        if rows:
            print(f"\n{label}:")
            for r in sorted(rows, key=lambda r: r["name"]):
                qty = f" ×{r['count']}" if r["count"] > 1 else ""
                print(f"  {r['name']}{qty}  [{dims(r['dims'])}]  in {', '.join(sorted(r['where']))}")


def _display(inst) -> str:
    # Anonymous Fusion component bodies ("=>[0:1:1:43]" -> SOLID) are named after their component.
    if inst.proto.name in ("SOLID", "COMPOUND", "") and len(inst.path) >= 2:
        return f"{inst.path[-2].split(':')[0]} (own body)"
    return _short(inst.proto.name)


def _pick(asm, pattern: str | None):
    """The one part whose name matches `pattern` (falling back to its assembly path)."""
    from .contacts import dedupe

    keep, _ = dedupe(asm.leaf_instances())
    named = {i.proto.entry: (_display(i), i.proto) for i in keep}
    if pattern is None:
        if len(named) != 1:
            sys.exit(f"{len(named)} parts in this file; choose one with --name. Parts:\n  " + "\n  ".join(sorted({n for n, _ in named.values()})))
        return next(iter(named.values()))
    rx = re.compile(pattern, re.I)
    hits = {e: v for e, v in named.items() if rx.search(v[0])}
    if not hits:
        hits = {i.proto.entry: named[i.proto.entry] for i in keep if rx.search("/".join(i.path))}
    if len(hits) != 1:
        sys.exit(f"--name {pattern!r} matched {len(hits)} parts: {sorted({n for n, _ in hits.values()})}")
    return next(iter(hits.values()))


def cmd_part(args) -> None:
    from .geom import analyse, axis_label
    from .render import Label, contact_sheet, tessellate
    from .step import load

    asm = load(args.step)
    title, proto = _pick(asm, args.name)
    r = analyse(title, proto.shape)

    print(f"# {title}")
    print(f"solids: {r.solids}   bbox: {_vec(r.size)} mm   volume: {r.volume / 1000:.1f} cm³   surface: {r.area / 100:.0f} cm²")
    print(f"centre of mass: {_vec(r.centre)}   (model frame, mm)")
    print("principal axes (smallest moment first ≈ long axis):")
    for ax, m in sorted(zip(r.principal_axes, r.principal_moments), key=lambda t: t[1]):
        print(f"  {axis_label(ax):>8}  I={m:.3g}")
    print("faces: " + ", ".join(f"{k} {v}" for k, v in sorted(r.face_types.items(), key=lambda t: -t[1])))
    print("\n## Axis features (holes / shafts; coaxial cylinders merged)")
    for f in r.axes:
        print(f"  {f.id:4} {axis_label(f.direction):>6} through {_vec(f.origin)}  {f.kind:5}  {f.describe()}")
    groups = Counter((axis_label(f.direction), f.describe()) for f in r.axes)
    pattern = [f"{n}× {d} along {ax}" for (ax, d), n in groups.items() if n > 1]
    if pattern:
        print("\n  repeated: " + "; ".join(pattern))
    print("\n## Largest flat faces (candidate mounting / print-bed faces)")
    for p in r.planes[: args.planes]:
        print(f"  outward {axis_label(p.normal):>8} at {p.offset:7.1f}   {p.area:7.0f} mm²  ({len(p.face_ids)} faces)")

    mesh = tessellate(proto.shape)
    col = np.tile([0.66, 0.6, 0.78], (len(mesh.tris), 1))
    holes = {i for f in r.axes if f.kind == "hole" for i in f.face_ids}
    bosses = {i for f in r.axes if f.kind != "hole" for i in f.face_ids}
    col[np.isin(mesh.face_of_tri, list(holes))] = [0.95, 0.55, 0.15]
    col[np.isin(mesh.face_of_tri, list(bosses))] = [0.35, 0.6, 0.9]
    labels = [Label(f.id, set(f.face_ids), axis=(f.origin, f.direction) if f.kind == "hole" else None) for f in r.axes]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    png = out / (re.sub(r"[^A-Za-z0-9]+", "-", title).strip("-") + ".png")
    views = ["iso", "iso-rear", "top (-Z)", "bottom (+Z)", "front (+Y)", "right (-X)"] if args.all_views else ["iso", "top (-Z)", "front (+Y)", "right (-X)"]
    contact_sheet(mesh, col, labels, f"{title}  (orange: holes, blue: shafts/bosses)", views).save(png)
    print(f"\ncontact sheet: {png}")


def _units(args):
    from .contacts import find_contacts, units
    from .step import load

    asm = load(args.step)
    us, dropped = units(asm, args.rigid)
    return us, dropped, find_contacts(us)


def cmd_contacts(args) -> None:
    from .contacts import candidate_dirs, free_directions
    from .geom import axis_label

    us, _, cs = _units(args)
    rx = re.compile(args.name, re.I) if args.name else None
    by_a = defaultdict(list)
    for c in cs:
        by_a[c.a].append(c)
    for i, u in enumerate(us):
        if rx and not (rx.search(u.name) or rx.search("/".join(u.path))):
            continue
        free = free_directions(i, set(range(len(us))) - {i}, cs, us, candidate_dirs(u))
        where = " / ".join(p.split(":")[0] for p in u.path[1:-1]) or "—"
        print(f"[{i}] {_short(u.name)}   in {where}")
        print(f"     free in assembly: {', '.join(axis_label(d) for d in free) or 'none (held)'}")
        for c in by_a.get(i, []):
            kind = "touch" if c.touching_kinds else f"gap {c.distance:.2f}mm"
            extra = f", overlaps {c.interference:.2f}mm" if c.interference > 0.05 else ""
            faces = ", ".join(f"{k}×{n}" for k, n in c.touching_kinds.items())
            print(f"       ↔ [{c.b}] {_short(us[c.b].name)[:44]:44} {kind}{extra}  {faces}")


def cmd_plan(args) -> None:
    from .contacts import disassemble
    from .geom import axis_label

    us, _, cs = _units(args)
    steps, stuck = disassemble(us, cs)
    print("Assembly order (reverse of greedy disassembly); 'insert along' is the direction a part travels to go in.\n")
    for n, s in enumerate(reversed(steps)):
        on = ", ".join(_short(us[b].name) for b in s.seated_on)
        how = "base" if not np.any(s.direction) else f"insert along {axis_label(-s.direction)}"
        print(f"{n + 1:3}. {_short(us[s.unit].name):40} {how:22} onto: {on or '—'}")
    if stuck:
        print(f"\n⚠ {len(stuck)} units could not be freed (interlocked, or a joint the planner doesn't model yet):")
        for i in stuck:
            print(f"     [{i}] {_short(us[i].name)}")


def cmd_index(args) -> None:
    import json

    from .index import build, changes, write

    variants = json.loads(VARIANTS.read_text())
    names = args.variant or list(variants)
    for name in names:
        if name not in variants:
            sys.exit(f"unknown variant {name!r}; known: {', '.join(variants)}")
        out = Path(args.out_dir) / f"{name}.json"
        previous = json.loads(out.read_text()) if out.exists() else None
        index = build(name, variants[name], REPO_ROOT, previous)
        write(index, out)
        n_files = len(index["print_files"])
        matched = sum(1 for f in index["print_files"] if f["match"])
        print(f"{name}: {len(index['parts'])} parts, {len(index['instances'])} instances, "
              f"{matched}/{n_files} print files matched → {out.relative_to(REPO_ROOT)}")
        if previous:
            diff = changes(previous, index)
            print("  " + ("\n  ".join(diff) if diff else "no changes since last index"))
        for f in index["print_files"]:
            if not f["match"]:
                print(f"  unmatched file: {f['path']}")
            elif f["match"]["quality"] != "exact":
                print(f"  {f['match']['quality']}: {f['path']} → {f['match']['part']}")
        bought = [pid for pid, p in index["parts"].items() if p["kind"] == "hardware"]
        print(f"  hardware (no print file): {', '.join(bought)}")


def cmd_export_web(args) -> None:
    import json

    from .webexport import export

    variants = json.loads(VARIANTS.read_text())
    names = args.variant or [n for n, v in variants.items() if "web" in v]
    for name in names:
        index_path = TOOL_ROOT / "index" / f"{name}.json"
        if not index_path.exists():
            sys.exit(f"no index for {name}; run `just cad index {name}` first")
        export(name, variants[name], json.loads(index_path.read_text()), REPO_ROOT)


def main() -> None:
    ap = argparse.ArgumentParser(prog="just cad", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("tree", help="assembly structure with quantities and duplicate warnings")
    p.add_argument("step")
    p.add_argument("--depth", type=int, default=3)
    p.set_defaults(fn=cmd_tree)

    p = sub.add_parser("part", help="geometry report + contact sheet for one part")
    p.add_argument("step")
    p.add_argument("--name", help="regex selecting the part when the file has several")
    p.add_argument("--out", default=str(DEFAULT_OUT))
    p.add_argument("--planes", type=int, default=8)
    p.add_argument("--all-views", action="store_true")
    p.set_defaults(fn=cmd_part)

    p = sub.add_parser("index", help="(re)build the committed parts index for hardware variants")
    p.add_argument("variant", nargs="*", help="variants from variants.json (default: all)")
    p.add_argument("--out-dir", default=str(TOOL_ROOT / "index"))
    p.set_defaults(fn=cmd_index)

    p = sub.add_parser("export-web", help="write the demo GLB(s) for variants with a `web` section")
    p.add_argument("variant", nargs="*")
    p.set_defaults(fn=cmd_export_web)

    p = sub.add_parser("diff", help="compare the parts of two assemblies (e.g. build variants)")
    p.add_argument("a")
    p.add_argument("b")
    p.set_defaults(fn=cmd_diff)

    for name, fn, hlp in (
        ("contacts", cmd_contacts, "what each part touches and which ways it can move"),
        ("plan", cmd_plan, "experimental: derive an assembly order from contacts"),
    ):
        p = sub.add_parser(name, help=hlp)
        p.add_argument("step")
        p.add_argument("--rigid", action="append", help=f"regex for sub-assemblies treated as one unit (default {DEFAULT_RIGID})")
        if name == "contacts":
            p.add_argument("--name", help="only report units matching this regex")
        p.set_defaults(fn=fn)

    args = ap.parse_args()
    if hasattr(args, "rigid") and args.rigid is None:
        args.rigid = DEFAULT_RIGID
    args.fn(args)
