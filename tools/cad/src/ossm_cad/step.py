"""Load STEP files through OCCT's XCAF layer, keeping the assembly structure.

A STEP assembly is a graph: *prototypes* (a unique solid or sub-assembly, e.g.
"M3x12 bolt") and *instances* (one placement of a prototype inside a parent).
Fusion exports one prototype per unique part, so instance counts give the BOM
quantities and instance transforms give the assembled pose.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from OCP.IFSelect import IFSelect_RetDone
from OCP.STEPCAFControl import STEPCAFControl_Reader
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDataStd import TDataStd_Name
from OCP.TDF import TDF_Label, TDF_LabelSequence, TDF_Tool
from OCP.TCollection import TCollection_AsciiString
from OCP.TDocStd import TDocStd_Document
from OCP.TopLoc import TopLoc_Location
from OCP.TopoDS import TopoDS_Shape
from OCP.Quantity import Quantity_Color
from OCP.XCAFDoc import XCAFDoc_ColorTool, XCAFDoc_ColorType, XCAFDoc_DocumentTool, XCAFDoc_ShapeTool

RGB = tuple[float, float, float]  # linear, as glTF baseColorFactor expects


@dataclass
class Prototype:
    """A unique shape definition (part or sub-assembly)."""

    entry: str
    name: str
    shape: TopoDS_Shape
    is_assembly: bool
    children: list[Instance] = field(default_factory=list)
    color: RGB | None = None  # colour set on the definition itself


@dataclass
class Instance:
    """One placement of a prototype. `world` is a 4x4 matrix in mm."""

    name: str
    proto: Prototype
    local: np.ndarray
    world: np.ndarray
    path: tuple[str, ...]
    color: RGB | None = None  # set on this placement, or inherited from a parent assembly


@dataclass
class Assembly:
    path: Path
    roots: list[Prototype]
    prototypes: dict[str, Prototype]

    def leaf_instances(self) -> list[Instance]:
        """Every placed solid (non-assembly) with its world transform."""
        out: list[Instance] = []

        def walk(proto: Prototype, world: np.ndarray, path: tuple[str, ...], color: RGB | None):
            for inst in proto.children:
                w = world @ inst.local
                # Nearest colour wins: this placement, then the sub-assembly it's in.
                c = inst.color or (inst.proto.color if inst.proto.is_assembly else None) or color
                placed = Instance(inst.name, inst.proto, inst.local, w, path + (inst.name,), c)
                if inst.proto.is_assembly:
                    walk(inst.proto, w, placed.path, c)
                else:
                    out.append(placed)

        for root in self.roots:
            if root.is_assembly:
                walk(root, np.eye(4), (root.name,), root.color)
            else:
                out.append(Instance(root.name, root, np.eye(4), np.eye(4), (root.name,)))
        return out

    def face_colors(self, inst: Instance) -> list[RGB | None]:
        """Colour of each face (in `geom.faces` order): face colour, else the part's own
        colour, else the placement's or a parent assembly's, else None."""
        from .geom import faces

        tool: XCAFDoc_ColorTool = self._colors  # type: ignore[attr-defined]
        fallback = inst.proto.color or inst.color
        out = []
        for f in faces(inst.proto.shape):
            c = Quantity_Color()
            if tool.GetColor(f, XCAFDoc_ColorType.XCAFDoc_ColorSurf, c) or tool.GetColor(
                f, XCAFDoc_ColorType.XCAFDoc_ColorGen, c
            ):
                out.append(_rgb(c))
            else:
                out.append(fallback)
        return out


def _entry(label: TDF_Label) -> str:
    s = TCollection_AsciiString()
    TDF_Tool.Entry_s(label, s)
    return s.ToCString()


def _name(label: TDF_Label) -> str:
    attr = TDataStd_Name()
    if label.FindAttribute(TDataStd_Name.GetID_s(), attr):
        return attr.Get().ToExtString()
    return ""


def _rgb(c: Quantity_Color) -> RGB:
    return (round(c.Red(), 4), round(c.Green(), 4), round(c.Blue(), 4))


def _label_color(label: TDF_Label) -> RGB | None:
    for kind in (XCAFDoc_ColorType.XCAFDoc_ColorSurf, XCAFDoc_ColorType.XCAFDoc_ColorGen):
        c = Quantity_Color()
        if XCAFDoc_ColorTool.GetColor_s(label, kind, c):
            return _rgb(c)
    return None


def _matrix(loc: TopLoc_Location) -> np.ndarray:
    t = loc.Transformation()
    m = np.eye(4)
    for r in range(3):
        for c in range(4):
            m[r, c] = t.Value(r + 1, c + 1)
    return m


# Fusion appends occurrence counters ("Bolt (3)", "Bolt v2:1"); strip them so
# instances of one prototype share a readable base name.
_SUFFIX = re.compile(r"(\s*\(\d+\)|:\d+)$")


def clean_name(name: str) -> str:
    prev = None
    while prev != name:
        prev, name = name, _SUFFIX.sub("", name).strip()
    return name


def load(path: str | Path) -> Assembly:
    path = Path(path)
    doc = TDocStd_Document(TCollection_ExtendedString("XmlOcaf"))
    reader = STEPCAFControl_Reader()
    reader.SetNameMode(True)
    reader.SetColorMode(True)
    if reader.ReadFile(str(path)) != IFSelect_RetDone:
        raise RuntimeError(f"failed to read STEP: {path}")
    reader.Transfer(doc)

    tool = XCAFDoc_DocumentTool.ShapeTool_s(doc.Main())
    protos: dict[str, Prototype] = {}

    def proto_for(label: TDF_Label) -> Prototype:
        key = _entry(label)
        if key in protos:
            return protos[key]
        is_asm = XCAFDoc_ShapeTool.IsAssembly_s(label)
        p = Prototype(key, clean_name(_name(label)), XCAFDoc_ShapeTool.GetShape_s(label), is_asm,
                      color=_label_color(label))
        protos[key] = p
        if is_asm:
            comps = TDF_LabelSequence()
            XCAFDoc_ShapeTool.GetComponents_s(label, comps, False)
            for i in range(1, comps.Length() + 1):
                comp = comps.Value(i)
                ref = TDF_Label()
                XCAFDoc_ShapeTool.GetReferredShape_s(comp, ref)
                child = proto_for(ref)
                local = _matrix(XCAFDoc_ShapeTool.GetLocation_s(comp))
                p.children.append(Instance(_name(comp) or child.name, child, local, local, (), _label_color(comp)))
        return p

    free = TDF_LabelSequence()
    tool.GetFreeShapes(free)
    roots = [proto_for(free.Value(i)) for i in range(1, free.Length() + 1)]
    # Keep the document alive alongside the shapes it owns.
    asm = Assembly(path, roots, protos)
    asm._doc = doc  # type: ignore[attr-defined]
    asm._colors = XCAFDoc_DocumentTool.ColorTool_s(doc.Main())  # type: ignore[attr-defined]
    return asm
