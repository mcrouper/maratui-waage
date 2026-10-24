"""Build the MaraTUI assembly STEP from the individual part STEPs.

usage: python3 assemble.py   (needs build123d; run from hardware/3d-print/)

Positions are derived from the part geometry (slot, tabs, mounting holes)
and the build photos; the load cells are simplified placeholders.
"""
import math, sys, itertools
import numpy as np
from OCP.gp import gp_Trsf
from build123d import (Align, import_step, export_step, Compound, Box, Cylinder, Location,
                       Rot, Pos, Color, GeomType, Vector)

P = "."
WINKEL, STREBE, OUT = "staender_winkel.stp", "strebe.stp", "maratui_baugruppe.step"

def M(rot, t=(0, 0, 0)):
    m = np.eye(4); m[:3, :3] = rot; m[:3, 3] = t; return m

def loc(m):
    tr = gp_Trsf()
    tr.SetValues(*m[0], *m[1], *m[2])
    return Location(tr)

def place(shape, m, label, color):
    s = shape.moved(loc(m)); s.label = label; s.color = Color(color); return s

# ---------------------------------------------------------------- display + stand
winkel = import_step(WINKEL)
strebe = import_step(STREBE)
body = import_step(f"{P}/geh_koerper_V3.stp")
lid = import_step(f"{P}/geh_deckel_V3.stp")

def face_near(shape, c, n):
    best = min((f for f in shape.faces() if f.geom_type == GeomType.PLANE),
               key=lambda f: (f.center() - Vector(*c)).length + 10 * (f.normal_at() - Vector(*n)).length)
    return best

# strebe -> slot of the winkel (all in the winkel frame W, profile in XZ)
floor = face_near(winkel, (42.236, 0, -8.768), (0.8870, 0, -0.4619))
wall_a = face_near(winkel, (43.848, 0, -12.709), (0.4619, 0, 0.8870))
wall_b = face_near(winkel, (46.388, 0, -7.830), (-0.4619, 0, -0.8870))
nf = np.array([floor.normal_at().X, floor.normal_at().Z])
nw = np.array([wall_a.normal_at().X, wall_a.normal_at().Z])
round_f = face_near(strebe, (36.855, 0, -10.349), (-0.9397, 0, 0.3420))
ns = np.array([round_f.normal_at().X, round_f.normal_at().Z])
ang = math.atan2(-nf[1], -nf[0]) - math.atan2(ns[1], ns[0])   # rounded face must face the slot floor
c, s = math.cos(ang), math.sin(ang)
R2 = np.array([[c, -s], [s, c]])                               # rotation in the (x,z) plane
def xz(f): v = f.center(); return np.array([v.X, v.Z])
side_a = face_near(strebe, (40.529, 0, -9.026), (0.3420, 0, 0.9397))
side_b = face_near(strebe, (38.819, 0, -13.725), (-0.3420, 0, -0.9397))
rf, sa, sb = R2 @ xz(round_f), R2 @ xz(side_a), R2 @ xz(side_b)
slot_mid = (xz(wall_a) + xz(wall_b)) / 2
# solve t: (rf+t).nf == floor.nf  and  ((sa+sb)/2+t).nw == slot_mid.nw
A = np.array([nf, nw]); b = np.array([(xz(floor) - rf) @ nf, (slot_mid - (sa + sb) / 2) @ nw])
t = np.linalg.solve(A, b)
m_strebe = M([[R2[0, 0], 0, R2[0, 1]], [0, 1, 0], [R2[1, 0], 0, R2[1, 1]]], (t[0], 0, t[1]))

# lid -> body frame: tabs fill the two side-wall gaps, wall ends butt at z=0
m_lid = M([[0, 1, 0], [1, 0, 0], [0, 0, -1]], (0, 0, -9))
# case -> W: back (z=30) flat on the upright's inner face (z_w=-39),
# bottom edge (y=-33) in the corner at x_w=27, display facing away from the upright
m_case = M([[0, -1, 0], [-1, 0, 0], [0, 0, -1]], (-6, 0, -9))

# W -> world: wedge underside on the table (Z up)
a = math.atan2(-13.8 + 43.2, 45.8 - 30.6)
ca, sa_ = math.cos(-a), math.sin(-a)
m_table = M([[ca, 0, -sa_], [0, 1, 0], [sa_, 0, ca]]) @ M(np.eye(3), (-30.6, 0, 43.2))

stand_parts = [
    (winkel, M(np.eye(3), (0, -32.5, 0)), "staender_winkel_links", "gray20"),
    (winkel, M(np.eye(3), (0, 32.5, 0)), "staender_winkel_rechts", "gray20"),
    (strebe, m_strebe, "strebe", "gray30"),
    (body, m_case, "geh_koerper_V3", "gray15"),
    (lid, m_case @ m_lid, "geh_deckel_V3", "gray25"),
]
tmp = [s.moved(loc(m_table @ m)) for s, m, *_ in stand_parts]
zmin = min(x.bounding_box().min.Z for x in tmp)
xmin = min(x.bounding_box().min.X for x in tmp)
m_disp = M(np.eye(3), (170 - xmin, -80, -zmin)) @ m_table
display = [place(s, m_disp @ m, l, col) for s, m, l, col in stand_parts]

# ---------------------------------------------------------------- scale
fuss = import_step(f"{P}/doppelfuss_V2.stp")
auflage = import_step(f"{P}/auflage_v2.stp")
# simplified 5 kg bar load cell 80 x 12.7 x 12.7, holes 5/20 mm from each end
cell = Box(80, 12.7, 12.7, align=(Align.MIN, Align.CENTER, Align.CENTER))
for x in (5, 20, 60, 75):
    cell = cell - Cylinder(1.75, 20).moved(Pos(x, 0, 0) * Rot(90, 0, 0))
for x in (35, 45):
    cell = cell - Cylinder(4.5, 20).moved(Pos(x, 0, 0))
cell = cell - Box(10, 3, 20).moved(Pos(40, 0, 0))
CELL_Y0 = 28.0                                   # top of the mounting blocks
m_cell = lambda zc: M(np.eye(3), (0, CELL_Y0 + 6.35, zc))
m_auf = lambda zc: M([[0, 1, 0], [0, 0, -1], [-1, 0, 0]], (67.5, CELL_Y0 + 12.7, zc))
m_scale = M([[1, 0, 0], [0, 0, -1], [0, 1, 0]])  # doppelfuss is modelled Y-up
scale = [
    place(fuss, m_scale, "doppelfuss_V2", "royalblue"),
    place(cell, m_scale @ m_cell(0), "waegezelle_1 (vereinfacht)", "silver"),
    place(cell, m_scale @ m_cell(160), "waegezelle_2 (vereinfacht)", "silver"),
    place(auflage, m_scale @ m_auf(0), "auflage_v2_1", "royalblue"),
    place(auflage, m_scale @ m_auf(160), "auflage_v2_2", "royalblue"),
]

# ---------------------------------------------------------------- checks + export
for grp in (display, scale):
    for p, q in itertools.combinations(grp, 2):
        try:
            v = (p & q).volume
        except Exception:
            v = float('nan')
        if v > 0.01:
            print(f"KOLLISION {p.label} <-> {q.label}: {v:.1f} mm3")
for x in display + scale:
    b = x.bounding_box()
    print(f"{x.label:28s} X {b.min.X:7.1f}..{b.max.X:7.1f}  Y {b.min.Y:7.1f}..{b.max.Y:7.1f}  Z {b.min.Z:6.1f}..{b.max.Z:6.1f}")

asm = Compound(label="maratui_baugruppe", children=[
    Compound(label="waage", children=scale),
    Compound(label="display_mit_staender", children=display),
])
export_step(asm, OUT)
print("written", OUT)
