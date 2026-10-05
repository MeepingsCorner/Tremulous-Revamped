"""Tremulous MD3 models (weapons, players, buildables) -> OBJ + MTL + PNG textures.

A standing pose of every model (frame 0 when it has no animation); Y-up (Quake's Z-up turned), one OBJ per .md3, plus
the human player assembled (lower + upper + head on their tags).

Usage: python tools/md3obj.py <folder with models/ and scripts/ unpacked from
the pk3s> <output folder>   (needs Pillow; made the owner's Desktop\Tremulous Models)."""
import math
import os
import re
import struct
import sys

from PIL import Image

SRC = sys.argv[1]
OUT = sys.argv[2]
KEEP = ("models/weapons/", "models/players/", "models/buildables/")


def cstr(b):
    return b.split(b"\0", 1)[0].decode("latin1")


def anim_cfg(model_dir):
    """The (first, num) frames of a model's animation.cfg, in file order."""
    p = os.path.join(model_dir, "animation.cfg")
    out = []
    if os.path.exists(p):
        for line in open(p, encoding="latin1"):
            nums = re.findall(r"-?\d+", line.split("//")[0])
            if len(nums) == 4 and line.split("//")[0].strip()[0].isdigit():
                out.append((int(nums[0]), int(nums[1])))
    return out


def pose_frame(md3_path):
    """A standing pose: humans TORSO_STAND / LEGS_IDLE (legs frames less the
    torso ones, as cg_players.c), aliens NSPA_STAND, structures IDLE1."""
    d, f = os.path.split(md3_path)
    a = anim_cfg(d)
    part = os.path.splitext(f)[0].lower()
    if "/players/" in md3_path.replace("\\", "/") and len(a) > 22:
        skip = a[13][0] - a[6][0]
        if part == "upper":
            return a[11][0]
        if part == "lower":
            return a[22][0] - skip
        return 0
    if "/buildables/" in md3_path.replace("\\", "/") and len(a) > 2 and a[2][1] > 0:
        return a[2][0]
    return a[0][0] if "/players/" in md3_path.replace("\\", "/") and a else 0


def read_md3(path, frame=None):
    d = open(path, "rb").read()
    if d[:4] != b"IDP3":
        return None
    (ver,) = struct.unpack_from("<i", d, 4)
    _flags, nframes, ntags, nsurf, nskins, ofs_frames, ofs_tags, ofs_surf, ofs_end = struct.unpack_from("<9i", d, 72)
    if frame is None:
        frame = pose_frame(path)
    frame = max(0, min(frame, nframes - 1))
    tags = {}
    for t in range(ntags):  # the pose frame's tags
        o = ofs_tags + (frame * ntags + t) * 112
        name = cstr(d[o : o + 64])
        org = struct.unpack_from("<3f", d, o + 64)
        axis = [struct.unpack_from("<3f", d, o + 76 + 12 * k) for k in range(3)]
        tags[name] = (org, axis)
    surfs = []
    o = ofs_surf
    for _ in range(nsurf):
        name = cstr(d[o + 4 : o + 68])
        flags, sf, nsh, nv, ntri, ofs_tri, ofs_sh, ofs_st, ofs_xyz, ofs_end_s = struct.unpack_from("<10i", d, o + 68)
        shaders = [cstr(d[o + ofs_sh + k * 68 : o + ofs_sh + k * 68 + 64]) for k in range(nsh)]
        tris = [struct.unpack_from("<3i", d, o + ofs_tri + 12 * k) for k in range(ntri)]
        st = [struct.unpack_from("<2f", d, o + ofs_st + 8 * k) for k in range(nv)]
        xyz, nrm = [], []
        for k in range(nv):
            x, y, z, n = struct.unpack_from("<3hH", d, o + ofs_xyz + 8 * (frame * nv + k))
            xyz.append((x / 64.0, y / 64.0, z / 64.0))
            lat = ((n >> 8) & 255) * 2 * math.pi / 255
            lng = (n & 255) * 2 * math.pi / 255
            nrm.append((math.cos(lat) * math.sin(lng), math.sin(lat) * math.sin(lng), math.cos(lng)))
        surfs.append({"name": name, "shader": shaders[0] if shaders else "", "tris": tris, "st": st, "xyz": xyz, "nrm": nrm})
        o += ofs_end_s
    return {"surfs": surfs, "tags": tags}


# shader scripts: shader name -> its first image stage
SHADER_IMG = {}
for f in sorted(os.listdir(os.path.join(SRC, "scripts"))):
    if not f.endswith(".shader"):
        continue
    txt = open(os.path.join(SRC, "scripts", f), encoding="latin1").read()
    txt = re.sub(r"//[^\n]*", "", txt)
    for m in re.finditer(r"([^\s{}]+)\s*\{((?:[^{}]|\{[^{}]*\})*)\}", txt):
        name, body = m.group(1).lower(), m.group(2)
        im = re.search(r"\b(?:map|clampmap|animmap\s+[\d.]+)\s+([^\s$][^\s]*)", body, re.I)
        if im:
            SHADER_IMG.setdefault(name, im.group(1))


def find_image(name):
    """A shader / image name -> an existing image file under SRC."""
    if not name:
        return None
    n = name.replace("\\", "/").lower().lstrip("/")
    cands = [n]
    if n in SHADER_IMG:
        cands.insert(0, SHADER_IMG[n].lower())
    stem = os.path.splitext(n)[0]
    if stem in SHADER_IMG:
        cands.insert(0, SHADER_IMG[stem].lower())
    for c in cands:
        s = os.path.splitext(c)[0]
        for ext in (".tga", ".jpg", ".png", ".jpeg"):
            p = os.path.join(SRC, s + ext)
            if os.path.exists(p):
                return p
    return None


def skin_map(md3_path):
    """surface -> image from <part>_default.skin (players), if any."""
    base = os.path.splitext(md3_path)[0]
    for sk in (base + "_default.skin",):
        if os.path.exists(sk):
            m = {}
            for line in open(sk, encoding="latin1"):
                if "," in line:
                    s, img = line.strip().split(",", 1)
                    m[s.strip().lower()] = img.strip()
            return m
    return {}


def to_png(img_path, out_dir, used):
    name = os.path.splitext(os.path.basename(img_path))[0] + ".png"
    if name not in used:
        Image.open(img_path).save(os.path.join(out_dir, name))
        used.add(name)
    return name


def yup(v):
    return (v[0], v[2], -v[1])


def write_obj(parts, out_dir, stem):
    """parts: [(md3, skin map, transform(origin, axis) or None)]."""
    os.makedirs(out_dir, exist_ok=True)
    used, mats = set(), {}
    obj = [f"# {stem}: Tremulous model, standing pose, units = Quake units, Y up", f"mtllib {stem}.mtl"]
    base = 1
    for md3, skins, xf in parts:
        for s in md3["surfs"]:
            if not s["tris"]:
                continue
            img = skins.get(s["name"].lower()) or s["shader"]
            if img and os.path.splitext(os.path.basename(img.lower()))[0] == "null":
                continue  # hidden surfaces of a skin
            path = find_image(img)
            mname = re.sub(r"[^A-Za-z0-9_]", "_", os.path.splitext(os.path.basename(img or s["name"]))[0]) or "mat"
            if mname not in mats:
                mats[mname] = to_png(path, out_dir, used) if path else None
            obj.append(f"o {s['name']}")
            obj.append(f"usemtl {mname}")
            for v, n in zip(s["xyz"], s["nrm"]):
                if xf:
                    org, ax = xf
                    v = tuple(org[k] + v[0] * ax[0][k] + v[1] * ax[1][k] + v[2] * ax[2][k] for k in range(3))
                    n = tuple(n[0] * ax[0][k] + n[1] * ax[1][k] + n[2] * ax[2][k] for k in range(3))
                x, y, z = yup(v)
                obj.append(f"v {x:.4f} {y:.4f} {z:.4f}")
            for t in s["st"]:
                obj.append(f"vt {t[0]:.6f} {1.0 - t[1]:.6f}")
            for n in s["nrm"] if not xf else [tuple(nn[0] * xf[1][0][k] + nn[1] * xf[1][1][k] + nn[2] * xf[1][2][k] for k in range(3)) for nn in s["nrm"]]:
                x, y, z = yup(n)
                obj.append(f"vn {x:.4f} {y:.4f} {z:.4f}")
            for a, b, c in s["tris"]:
                # Quake winds clockwise: reversed for OBJ's counter-clockwise front
                ia, ib, ic = a + base, c + base, b + base
                obj.append(f"f {ia}/{ia}/{ia} {ib}/{ib}/{ib} {ic}/{ic}/{ic}")
            base += len(s["xyz"])
    open(os.path.join(out_dir, stem + ".obj"), "w").write("\n".join(obj) + "\n")
    mtl = []
    for m, tex in mats.items():
        mtl += [f"newmtl {m}", "Ka 0 0 0", "Kd 1 1 1", "Ks 0 0 0", "d 1", "illum 1"]
        if tex:
            mtl.append(f"map_Kd {tex}")
        mtl.append("")
    open(os.path.join(out_dir, stem + ".mtl"), "w").write("\n".join(mtl))
    return len(mats)


def compose(t_parent, t_child):
    """child tag (org, axis) expressed in the parent's frame -> world."""
    po, pa = t_parent
    co, ca = t_child
    org = tuple(po[k] + co[0] * pa[0][k] + co[1] * pa[1][k] + co[2] * pa[2][k] for k in range(3))
    axis = [tuple(ca[i][0] * pa[0][k] + ca[i][1] * pa[1][k] + ca[i][2] * pa[2][k] for k in range(3)) for i in range(3)]
    return (org, axis)


count = 0
for root, _, files in os.walk(os.path.join(SRC, "models")):
    rel = os.path.relpath(root, SRC).replace("\\", "/") + "/"
    if not rel.startswith(KEEP):
        continue
    for f in sorted(files):
        if not f.lower().endswith(".md3"):
            continue
        p = os.path.join(root, f)
        md3 = read_md3(p)
        if not md3 or not any(s["tris"] for s in md3["surfs"]):
            continue  # the *_hand.md3: tags only
        cat, model = rel.split("/")[1], rel.split("/")[2]
        out_dir = os.path.join(OUT, cat.capitalize(), model)
        n = write_obj([(md3, skin_map(p), None)], out_dir, os.path.splitext(f)[0])
        count += 1
        print(f"{cat}/{model}/{f}: {len(md3['surfs'])} surfaces, {n} materials")

# the humans assembled: lower, upper on tag_torso, head on upper's tag_head
for model in ("human_base", "human_bsuit"):
    d = os.path.join(SRC, "models/players", model)
    if not os.path.exists(os.path.join(d, "lower.md3")):
        continue
    lower, upper, head = (read_md3(os.path.join(d, x + ".md3")) for x in ("lower", "upper", "head"))
    ident = ((0.0, 0.0, 0.0), [(1.0, 0, 0), (0, 1.0, 0), (0, 0, 1.0)])
    t_torso = compose(ident, lower["tags"]["tag_torso"])
    t_head = compose(t_torso, upper["tags"]["tag_head"])
    parts = [(lower, skin_map(os.path.join(d, "lower.md3")), None), (upper, skin_map(os.path.join(d, "upper.md3")), t_torso), (head, skin_map(os.path.join(d, "head.md3")), t_head)]
    write_obj(parts, os.path.join(OUT, "Players", model), model + "_assembled")
    count += 1
    print(f"players/{model}: assembled")
print("models:", count)
