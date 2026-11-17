# CAD tools

Reads the upstream hardware CAD (STEP, STL, 3MF under `reference/`) and derives
the data our build guide and demos need. Upstream stays the source of truth for
anything printable. We index it, link to it, and make lightweight demo meshes.

Run through `just cad <command>` (the nix dev shell provides `uv` and the
shared libraries the OCP wheels need):

| command | |
|---|---|
| `index [variant…]` | (Re)build `index/<variant>.json` for the variants in `variants.json`, and report what changed since the last run |
| `export-web [variant…]` | Write the demo GLB for variants with a `web` section (the site's `ossm-alt.glb`) |
| `tree <step>` | Assembly structure, quantities, duplicate warnings |
| `part <step> --name X` | Geometry report and a labelled multi-view contact sheet (`out/`) |
| `diff <a.step> <b.step>` | Compare two builds' parts |
| `contacts <step>` | What each part touches and which ways it can slide free |
| `plan <step>` | Experimental: assembly order from contacts |

## The index

`index/<variant>.json` is generated. Don't edit it by hand. It holds:

- `parts`: unique solids keyed by a stable id, with a pose-independent
  fingerprint (volume, area, principal moments), kind
  (`printed`/`fastener`/`hardware`/`electronics`), hole summary and matched print files;
- `instances`: every placement (`<part>#n`), assembly path and 3×4 transform in mm;
- `assembly_units`: parts that move together, what they touch, and free directions;
- `print_files`: upstream STL/3MF/STEP files with permalinks pinned to the
  submodule commit, matched to parts by fingerprint (`exact`, or `variant`
  for a different revision or print option).

After bumping the submodule, run `just cad index`. Ids are carried over from the
previous index (by geometry, then name) and the output lists added, removed,
changed and moved parts, plus print files that were added or rematched.

## Demo meshes

`export-web` turns an index into the site's model. It's demo fidelity: coarse
tessellation scaled to part size, PCB components drawn as boxes, each unique
part stored once and placed by nodes named with index instance ids (which
`apps/web-tools/src/parts.ts` refers to). `variants.json` → `web` sets the
display frame (metres, Y up), parts to leave out, groups (the simulator moves
`rail` and keeps `housing` still) and an extras file.

## Parts upstream doesn't model

The index adds these as ordinary parts and instances, with a `source` and a
`shape` spec that the exporter rebuilds geometry from:

- Nuts (`source: derived: nut pocket`): hex pockets and side-entry slots in
  printed parts, around holes that take that thread, each get a standard ISO
  nut, seated at the end facing the bolt's bore.
- `variants.json` → `additions` (`source: authored`): standard bolts (snapped
  onto the nearest hole axis), a part from another upstream STEP, the odd nut
  with no pocket, and the GT2 belt. Its centreline is
  computed from the pulley, the idler bearing stacks and the two clamp ends.
