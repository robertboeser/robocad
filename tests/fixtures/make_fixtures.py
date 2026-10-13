#!/usr/bin/env python3
"""Generates the small STL test fixtures in this directory. Run: python3 make_fixtures.py"""
import math
import struct

def cube(o=(0, 0, 0), s=10.0):
    x, y, z = o
    c = [(x + s * (i & 1), y + s * ((i >> 1) & 1), z + s * ((i >> 2) & 1)) for i in range(8)]
    idx = [(0, 2, 1), (1, 2, 3), (4, 5, 6), (5, 7, 6), (0, 1, 4), (1, 5, 4),
           (2, 6, 3), (3, 6, 7), (0, 4, 2), (2, 4, 6), (1, 3, 5), (3, 7, 5)]
    return [(c[a], c[b], c[d]) for a, b, d in idx]

def tet(apex_down=False, o=(0, 0, 0)):
    x, y, z = o
    if apex_down:
        p = [(x, y, z), (x - 5, y - 5, z + 10), (x + 5, y - 5, z + 10), (x, y + 5, z + 10)]
        return [(p[0], p[2], p[1]), (p[0], p[3], p[2]), (p[0], p[1], p[3]), (p[1], p[2], p[3])]
    p = [(x, y, z), (x - 5, y - 5, z - 10), (x + 5, y - 5, z - 10), (x, y + 5, z - 10)]
    return [(p[0], p[1], p[2]), (p[0], p[2], p[3]), (p[0], p[3], p[1]), (p[1], p[3], p[2])]

def normal(t):
    a, b, c = t
    u = [b[i] - a[i] for i in range(3)]
    v = [c[i] - a[i] for i in range(3)]
    n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    l = math.sqrt(sum(k * k for k in n)) or 1.0
    return [k / l for k in n]

def write_binary(path, tris, header=b"robocad fixture", count=None, extra=b""):
    with open(path, "wb") as f:
        f.write(header.ljust(80, b"\0")[:80])
        f.write(struct.pack("<I", len(tris) if count is None else count))
        for t in tris:
            f.write(struct.pack("<3f", *normal(t)))
            for v in t:
                f.write(struct.pack("<3f", *v))
            f.write(b"\0\0")
        f.write(extra)

def write_ascii(path, solids, upper=False):
    lines = []
    for name, tris in solids:
        lines.append(f"solid {name}")
        for t in tris:
            n = normal(t)
            lines.append(f"  facet normal {n[0]:e} {n[1]:e} {n[2]:e}")
            lines.append("\touter loop")
            for v in t:
                lines.append(f"\t\tvertex   {v[0]:.6e} {v[1]:.6e} {v[2]:.6e}")
            lines.append("\tendloop")
            lines.append("  endfacet")
        lines.append(f"endsolid {name}")
    text = "\r\n".join(lines) + "\r\n"
    if upper:
        text = text.upper()
    with open(path, "w", newline="") as f:
        f.write(text)

c = cube()
write_ascii("cube_ascii.stl", [("cube", c)])
write_binary("cube_binary.stl", c)
write_binary("cube_binary_solid_header.stl", c, header=b"solid exported by SomeCAD")
write_binary("cube_missing_triangle.stl", c[:-1])
write_binary("two_overlapping_cubes.stl", c + cube((5, 5, 5)))
write_binary("bowtie_vertex.stl", tet(False) + tet(True))
write_binary("shared_edge_cubes.stl", c + cube((10, 10, 0)))
flipped = list(c)
a, b, d = flipped[0]
flipped[0] = (a, d, b)
write_binary("cube_flipped_triangle.stl", flipped)
write_binary("cube_inverted.stl", [(a, d, b) for a, b, d in c])
write_binary("cube_degenerate.stl", c + [((0, 0, 0), (5, 0, 0), (10, 0, 0))])
nan = float("nan")
write_binary("cube_with_nan.stl", c + [((nan, 0, 0), (1, 0, 0), (0, 1, 0))])
write_binary("truncated.stl", c[:5], count=12)
write_binary("trailing_bytes.stl", c, extra=b"\0" * 7)
write_ascii("multi_solid_upper.stl", [("a", c), ("b", cube((20, 0, 0)))], upper=True)
open("empty.stl", "wb").close()
