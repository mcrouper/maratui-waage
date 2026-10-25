import sys
import numpy as np
import pyvista as pv
from build123d import import_step

# usage: python3 render.py   (needs build123d + pyvista; run from hardware/3d-print/)
STEP, SCREEN, OUT = "maratui_baugruppe.step", "../../assets/docs/extraction_screen.png", "../renders"
asm = import_step(STEP)
MAT = {  # label prefix -> (color, specular, power)
    "doppelfuss": ("#2f6fe0", 0.25, 20), "auflage": ("#2f6fe0", 0.25, 20),
    "waegezelle": ("#c9ccd1", 0.8, 60),
    "geh_": ("#26282b", 0.2, 15), "staender": ("#1c1d1f", 0.2, 15), "strebe": ("#1c1d1f", 0.2, 15),
}
parts = []
for leaf in asm.leaves:
    v, t = leaf.tessellate(0.05, 0.2)
    pts = np.array([tuple(p) for p in v]); tri = np.array(t)
    mesh = pv.PolyData(pts, np.hstack([np.full((len(tri), 1), 3), tri]).ravel())
    key = next(k for k in MAT if leaf.label.startswith(k))
    parts.append((leaf.label, mesh, MAT[key]))
    if leaf.label.startswith("geh_deckel"):
        tr = leaf.location.wrapped.Transformation()
        LID = np.array([[tr.Value(i, j) for j in range(1, 5)] for i in range(1, 4)])

# display image behind the bezel window (lid coords: x = up, -y = right)
def lidpt(x, y, z=4.9): return LID[:, :3] @ np.array([x, y, z]) + LID[:, 3]
quad = pv.PolyData(np.array([lidpt(23, 33), lidpt(23, -28), lidpt(-23, -28), lidpt(-23, 33)]),
                   np.array([4, 0, 1, 2, 3]))
quad.active_texture_coordinates = np.array([[0, 1], [1, 1], [1, 0], [0, 0]], float)
screen_tex = pv.read_texture(SCREEN)
FRONT = LID[:, 2]
DISP_Y = LID[1, 3]                      # display normal in world coords

def shot(name, cam, explode=None, only=None, size=(1800, 1100), screen=True):
    p = pv.Plotter(off_screen=True, window_size=size, lighting="none")
    p.set_background("#c9ced6", top="#ffffff")
    for label, m, (col, spec, pw) in parts:
        if only and not label.startswith(only):
            continue
        off = np.zeros(3)
        if explode:
            off = np.array(explode.get(label, (0, 0, 0)), float)
            if label == "staender_winkel":   # push the two brackets apart sideways
                off = off * np.sign(m.center[1] - DISP_Y)
        p.add_mesh(m.translate(off, inplace=False), color=col, specular=spec, specular_power=pw,
                   smooth_shading=True, split_sharp_edges=True, feature_angle=35)
        if screen and label.startswith("geh_deckel"):
            p.add_mesh(quad.translate(off - FRONT * 0.0, inplace=False), texture=screen_tex,
                       ambient=0.9, diffuse=0.2, lighting=True)
    p.add_light(pv.Light(position=(250, 150, 500), focal_point=(150, -80, 0), intensity=0.85))
    p.add_light(pv.Light(position=(-300, -400, 250), focal_point=(150, -80, 0), intensity=0.35))
    p.add_light(pv.Light(light_type="headlight", intensity=0.25))
    p.enable_ssao(radius=4, bias=0.05, kernel_size=128, blur=True); p.enable_anti_aliasing("fxaa")
    p.camera_position = cam
    p.screenshot(f"{OUT}/{name}.png")
    p.close(); print("wrote", name)

# camera: (position, focal point, view-up)
shot("baugruppe_01_gesamt", [(420, 230, 300), (130, -85, 25), (0, 0, 1)])
shot("baugruppe_02_display_front", [(330, -40, 120), (205, -80, 42), (0, 0, 1)], only=("geh", "staender", "strebe"))
shot("baugruppe_03_waage", [(-120, 160, 210), (60, -80, 25), (0, 0, 1)], only=("doppelfuss", "waegezelle", "auflage"), screen=False)
ex = {"geh_deckel_V3": tuple(FRONT * 35), "staender_winkel": (0, 30, -2), "strebe": (25, 0, -2),
      "geh_koerper_V3": (0, 0, 15), "auflage_v2": (0, 0, 45), "waegezelle_5kg_vereinfacht": (0, 0, 20)}
shot("baugruppe_04_explosion", [(470, 260, 330), (130, -85, 45), (0, 0, 1)], explode=ex)
