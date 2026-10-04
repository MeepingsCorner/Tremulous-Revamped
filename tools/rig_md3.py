#!/usr/bin/env python3
"""rig_md3.py - ragdoll skeletons for the player models (enhanced mode).

The MD3 models are vertex-animated: no skeleton. Each model's own animation
says which vertices move together, so the skeleton is recovered from it
(rigid skinning decomposition): k-means on the vertex trajectories, then
alternate (a) the best rigid transform of each group in every frame (Kabsch)
and (b) giving each vertex to the group that reproduces it best. Groups that
share triangle edges are linked into a tree (maximum shared edges, from the
largest group); the joint between two linked groups is the point both carry
alike in every frame (least squares, held near their shared boundary). The
joint's swing limit is the largest bend the animation shows, plus 25 %.

Humans are three MD3s (lower, upper, head) joined at tag_torso and
tag_head: the lower and upper parts are decomposed separately; the head is
one bone; the parts are joined at the tags of the standing frames. Their
joints are anatomical: the lower part is cut into pelvis, thighs, shins and
feet (seeded by height and side, refined by the animation), the upper part's
groups are named by the tree (torso, arms of two or more segments, the last
the forearm). Knees and elbows are hinges about the axis they bend on,
limited to the range a human joint allows (a knee never bends forward);
hips, shoulders, ankles, waist and neck are cones of anatomical size.

    python3 tools/rig_md3.py PK3 OUT.txt

OUT.txt, per model: its rest frame per part, its bones (name, parent, part,
joint position, swing limit in degrees, then optionally the joint's kind:
"cone SWING TWIST", or "hinge AX AY AZ MIN MAX TX TY TZ": the hinge axis,
the flexion range (degrees: the signed angle, about the axis, from the
parent's direction (its joint to this joint) to this bone's (this joint to
its tip point T))) and the bone of every vertex of
every surface; positions are in the MD3 space of the rest frames (the human
upper and head parts placed in the lower part's space by the tags). Only
these numbers are kept, none of the model data.
"""
import struct
import sys
import zipfile

import numpy as np

# (model dir, bones per part, rest frames): rest = a standing frame
MODELS = {
    "human_base": {"lower": 7, "upper": 6},
    "human_bsuit": {"lower": 7, "upper": 8},  # its arms need 8 to come apart from the torso
    "builder": {"nonseg": 9},
    "builderupg": {"nonseg": 9},
    "level0": {"nonseg": 8},
    "level1": {"nonseg": 10},
    "level1upg": {"nonseg": 10},
    "level2": {"nonseg": 10},
    "level2upg": {"nonseg": 10},
    "level3": {"nonseg": 12},
    "level3upg": {"nonseg": 12},
    "level4": {"nonseg": 14},
}


def parse(z, name):
    b = z.read(name)
    nf, nt, ns, nsk, ofs_frames, ofs_tags, ofs_surf, ofs_end = struct.unpack_from("<iiiiiiii", b, 76)
    tags = []
    for f in range(nf):
        tf = {}
        for t in range(nt):
            o = ofs_tags + (f * nt + t) * 112
            nm = b[o:o + 64].split(b"\0")[0].decode().lower()
            v = struct.unpack_from("<12f", b, o + 64)
            tf[nm] = (np.array(v[0:3]), np.array(v[3:12]).reshape(3, 3))
        tags.append(tf)
    surfs = []
    o = ofs_surf
    for s in range(ns):
        flags, snf, nsh, nv, ntri, ofs_tri, ofs_sh, ofs_st, ofs_xyz, ofs_e = struct.unpack_from("<10i", b, o + 68)
        tri = np.frombuffer(b, "<i4", ntri * 3, o + ofs_tri).reshape(-1, 3)
        xyz = np.frombuffer(b, "<i2", snf * nv * 4, o + ofs_xyz).reshape(snf, nv, 4)[:, :, :3].astype(np.float64) / 64.0
        surfs.append((tri, xyz))
        o += ofs_e
    return nf, tags, surfs


def anim_frames(z, d):
    """animation.cfg: [(first, num, name)], legs frames adjusted as CG_ParseAnimationFile."""
    out = []
    for line in z.read(f"models/players/{d}/animation.cfg").decode("latin1").splitlines():
        p = line.split("//")[0].split()
        if len(p) >= 4 and p[0].lstrip("-").isdigit():
            out.append([int(p[0]), int(p[1]), line.split("//")[-1].strip().split()[0] if "//" in line else ""])
    names = [a[2] for a in out]
    if "LEGS_WALKCR" in names and "TORSO_GESTURE" in names:
        skip = out[names.index("LEGS_WALKCR")][0] - out[names.index("TORSO_GESTURE")][0]
        for a in out[names.index("LEGS_WALKCR"):]:
            a[0] -= skip
    return {a[2]: a for a in out}


def kabsch(P, Q):
    cp, cq = P.mean(0), Q.mean(0)
    U, S, Vt = np.linalg.svd((P - cp).T @ (Q - cq))
    d = np.sign(np.linalg.det(Vt.T @ U.T))
    R = Vt.T @ np.diag([1, 1, d]) @ U.T
    return R, cq - R @ cp


def weld(xyz, rest):
    key = np.round(xyz[rest] * 8).astype(np.int64)
    _, idx, inv = np.unique(key, axis=0, return_index=True, return_inverse=True)
    return idx, inv.reshape(-1)


def decompose(frames, rest, k, seed, iters=30):
    T, n, _ = frames.shape
    rng = np.random.default_rng(seed)
    traj = frames.transpose(1, 0, 2).reshape(n, -1)
    c = traj[rng.choice(n, k, replace=False)].copy()
    for _ in range(20):
        lab = ((traj[:, None, :] - c[None]) ** 2).sum(-1).argmin(1)
        for j in range(k):
            if (lab == j).any():
                c[j] = traj[lab == j].mean(0)
    P = frames[rest]
    for _ in range(iters):
        Rs = np.tile(np.eye(3), (T, k, 1, 1))
        ts = np.zeros((T, k, 3))
        for j in range(k):
            m = lab == j
            if m.sum() < 3:
                continue
            for f in range(T):
                Rs[f, j], ts[f, j] = kabsch(P[m], frames[f, m])
        pred = np.einsum("tkab,nb->tkna", Rs, P) + ts[:, :, None, :]
        err = ((pred - frames[:, None]) ** 2).sum(-1).mean(0)
        for j in range(k):  # a group too small to fit takes no vertices
            if (lab == j).sum() < 3:
                err[j] = np.inf
        new = err.argmin(0)
        if (new == lab).all():
            break
        lab = new
    return lab, Rs, ts, np.sqrt(err.min(0))


def solve_joint(frames, rest, tris, lab, Rs, ts, a, b):
    """The point bones a and b carry alike in every frame, held near their seam."""
    P = frames[rest]
    cen_b = P[lab == b].mean(0)
    cen_a = P[lab == a].mean(0)
    bd = [v for t in tris for v in t if lab[v] == b and (lab[t] == a).any()]
    near = P[bd].mean(0) if bd else (cen_a + cen_b) / 2
    # (R_a - R_b) p = t_b - t_a in every frame, held near the boundary:
    # a weak pull (0.3) where the animation pins it, the boundary where it does not
    M = (Rs[:, a] - Rs[:, b]).reshape(-1, 3)
    y = (ts[:, b] - ts[:, a]).reshape(-1)
    lam = 0.3 * len(frames)
    p = np.linalg.solve(M.T @ M + lam * np.eye(3), M.T @ y + lam * near)
    if np.linalg.norm(p - near) > 0.6 * np.linalg.norm(cen_b - near) + 2.0:
        p = near  # an unpinned joint stays on the seam
    return p


def refine(frames, rest, lab, k, iters=30):
    """decompose's alternation from given labels (no k-means)."""
    T = len(frames)
    P = frames[rest]
    for _ in range(iters):
        Rs = np.tile(np.eye(3), (T, k, 1, 1))
        ts = np.zeros((T, k, 3))
        for j in range(k):
            m = lab == j
            if m.sum() < 3:
                continue
            for f in range(T):
                Rs[f, j], ts[f, j] = kabsch(P[m], frames[f, m])
        pred = np.einsum("tkab,nb->tkna", Rs, P) + ts[:, :, None, :]
        err = ((pred - frames[:, None]) ** 2).sum(-1).mean(0)
        for j in range(k):
            if (lab == j).sum() < 3:
                err[j] = np.inf
        new = err.argmin(0)
        if (new == lab).all():
            break
        lab = new
    return lab, Rs, ts, np.sqrt(err.min(0))


def flexion(pd, cd, axis):
    """The signed angle (degrees) from pd to cd about axis."""
    pd = pd - axis * (pd @ axis)
    cd = cd - axis * (cd @ axis)
    return float(np.degrees(np.arctan2(axis @ np.cross(pd, cd), pd @ cd)))


def unit(v):
    return v / max(np.linalg.norm(v), 1e-9)


# anatomical ranges (degrees): flexion of the hinges, from straight; cones
# (swing, twist) about the death pose
KNEE = (0.0, 155.0)  # straight at most (the solver gives a few degrees)
ELBOW = (0.0, 150.0)
ANKLE = (30.0, 15.0)  # a cone: the foot is too short a group to hinge reliably
HIP = (60.0, 20.0)
SHOULDER = (75.0, 40.0)
UPPER_ARM_MID = (45.0, 30.0)
TORSO_PIECE = (12.0, 10.0)
WAIST = (30.0, 20.0)
NECK = (35.0, 45.0)


def human_lower(frames, rest, tris):
    """Pelvis 0, then per leg (left +Y first) thigh, shin, foot: seeded by
    height (fractions of the standing model) and side, refined by the
    animation. Joints pinned by the animation; the knee and ankle hinge
    about the leg's sideways axis, its sign so that the knee bends back."""
    P = frames[rest]
    z0, z1 = P[:, 2].min(), P[:, 2].max()
    h = z1 - z0
    lab = np.zeros(len(P), int)
    for left, base in ((True, 1), (False, 4)):
        m = (P[:, 1] > 0) == left
        lab[m & (P[:, 2] < z0 + 0.80 * h)] = base
        lab[m & (P[:, 2] < z0 + 0.47 * h)] = base + 1
        lab[m & (P[:, 2] < z0 + 0.14 * h)] = base + 2
    lab, Rs, ts, e = refine(frames, rest, lab, 7)
    parent = [-1, 0, 1, 2, 0, 4, 5]
    joints = [P[lab == 0].mean(0)] + [solve_joint(frames, rest, tris, lab, Rs, ts, parent[b], b) for b in range(1, 7)]
    kinds = [None] * 7
    T = len(frames)
    for thigh in (1, 4):
        shin, foot = thigh + 1, thigh + 2
        kinds[thigh] = ("cone",) + HIP
        hip, knee, ankle = joints[thigh], joints[shin], joints[foot]
        toe = P[lab == foot].mean(0)
        td = unit(knee - hip)
        axis = unit(np.array([0.0, 1.0, 0.0]) - td * td[1])
        # the knee bends back in the animation: flexion positive (else flip)
        def at(f, b, p):
            return Rs[f, b] @ p + ts[f, b]
        fl = [flexion(at(f, shin, knee) - at(f, thigh, hip), at(f, shin, ankle) - at(f, shin, knee), Rs[f, thigh] @ axis) for f in range(T)]
        if np.median(fl) < 0:
            axis = -axis
            fl = [-x for x in fl]
        kinds[shin] = ("hinge", axis, KNEE[0], KNEE[1], ankle)
        kinds[foot] = ("cone",) + ANKLE
        print(f"  knee {thigh}: animation flexion {min(fl):.0f}..{max(fl):.0f}", flush=True)
    return dict(lab=lab, parent=parent, joints=joints, limits=[0.0] + [110.0] * 6, kinds=kinds, err=e, root=0, k=7)


def human_upper_kinds(r, frames, rest):
    """Name the upper part's groups by the tree: the root is the torso;
    a chain of two or more groups from it is an arm (the first is the
    shoulder, the last the forearm, hinged at the elbow on the axis it
    bends about in the animation); a single group is a piece of the torso."""
    k, parent, joints, lab = r["k"], r["parent"], r["joints"], r["lab"]
    P = frames[rest]
    kids = {b: [c for c in range(k) if parent[c] == b] for b in range(k)}
    kinds = [None] * k
    root = r["root"]
    for c in kids[root]:
        chain = [c]
        while len(kids[chain[-1]]) == 1:
            chain.append(kids[chain[-1]][0])
        if len(chain) == 1:
            kinds[c] = ("cone",) + (TORSO_PIECE if not kids[c] else UPPER_ARM_MID)
            continue
        kinds[chain[0]] = ("cone",) + SHOULDER
        for m in chain[1:-1]:
            kinds[m] = ("cone",) + UPPER_ARM_MID
        fore, up = chain[-1], chain[-2]
        elbow = joints[fore]
        tip = P[lab == fore].mean(0)
        sh = joints[up]
        Rs, ts = r["Rs"], r["ts"]
        # the axis the elbow bends about, averaged in the upper arm's frame
        acc = np.zeros(3)
        for f in range(len(frames)):
            ud = Rs[f, fore] @ elbow + ts[f, fore] - (Rs[f, up] @ sh + ts[f, up])
            fd = Rs[f, fore] @ tip + ts[f, fore] - (Rs[f, fore] @ elbow + ts[f, fore])
            acc += Rs[f, up].T @ np.cross(unit(ud), unit(fd))
        axis = unit(acc)
        fr = flexion(elbow - sh, tip - elbow, axis)
        kinds[fore] = ("hinge", axis, ELBOW[0], ELBOW[1], tip)
        fe = [flexion(Rs[f, fore] @ elbow + ts[f, fore] - (Rs[f, up] @ sh + ts[f, up]), Rs[f, fore] @ (tip - elbow), Rs[f, up] @ axis) for f in range(len(frames))]
        print(f"  elbow {fore}: standing flexion {fr:.0f}, animation {min(fe):.0f}..{max(fe):.0f}", flush=True)
        for g in kids[fore]:
            kinds[g] = ("cone",) + UPPER_ARM_MID
    for b in range(k):
        if kinds[b] is None and b != root:
            kinds[b] = ("cone",) + UPPER_ARM_MID
    return kinds


def tree(frames, rest, tris, k, seed):
    lab, Rs, ts, e = decompose(frames, rest, k, seed)
    used = sorted(set(lab.tolist()))
    remap = {o: i for i, o in enumerate(used)}
    lab = np.array([remap[l] for l in lab])
    Rs, ts, k = Rs[:, used], ts[:, used], len(used)
    P = frames[rest]
    A = np.zeros((k, k), int)
    for t in tris:
        l = lab[t]
        for i in range(3):
            if l[i] != l[(i + 1) % 3]:
                A[l[i], l[(i + 1) % 3]] += 1
    mass = np.bincount(lab, minlength=k)
    root = int(mass.argmax())
    parent = [-1] * k
    joints = [None] * k
    limits = [0.0] * k
    inside = {root}
    cen = [P[lab == j].mean(0) for j in range(k)]
    while len(inside) < k:
        cand = [(A[a, b], a, b) for a in inside for b in range(k) if b not in inside and A[a, b] > 0]
        if cand:
            _, a, b = max(cand)
        else:
            _, a, b = min((np.linalg.norm(cen[a] - cen[b]), a, b) for a in inside for b in range(k) if b not in inside)
        bd = [v for t in tris for v in t if lab[v] == b and (lab[t] == a).any()]
        near = P[bd].mean(0) if bd else (cen[a] + cen[b]) / 2
        # (R_a - R_b) p = t_b - t_a in every frame, held near the boundary:
        # a weak pull (0.3) where the animation pins it, the boundary where it does not
        M = (Rs[:, a] - Rs[:, b]).reshape(-1, 3)
        y = (ts[:, b] - ts[:, a]).reshape(-1)
        lam = 0.3 * len(frames)
        p = np.linalg.solve(M.T @ M + lam * np.eye(3), M.T @ y + lam * near)
        if np.linalg.norm(p - near) > 0.6 * np.linalg.norm(cen[b] - near) + 2.0:
            p = near  # an unpinned joint stays on the seam
        # the largest bend between the two over the animation
        R0 = Rs[rest, a].T @ Rs[rest, b]
        ang = [np.degrees(np.arccos(np.clip((np.trace((Rs[f, a].T @ Rs[f, b]) @ R0.T) - 1) / 2, -1, 1))) for f in range(len(frames))]
        parent[b], joints[b], limits[b] = a, p, float(np.clip(max(ang) * 1.25, 15.0, 110.0))
        inside.add(b)
    joints[root] = cen[root]
    return dict(lab=lab, parent=parent, joints=joints, limits=limits, err=e, root=root, k=k, Rs=Rs, ts=ts)


def part_rig(z, path, rest, k, human_lower_part=False):
    nf, tags, surfs = parse(z, path)
    xyz = np.concatenate([s[1] for s in surfs], axis=1)
    off = np.cumsum([0] + [s[1].shape[1] for s in surfs])
    tris = np.concatenate([s[0] + off[i] for i, s in enumerate(surfs)])
    if k == 1:
        return dict(lab=np.zeros(xyz.shape[1], int), parent=[-1], joints=[xyz[rest].mean(0)], limits=[0.0], err=np.zeros(1), root=0, k=1), [s[1].shape[1] for s in surfs], tags, 0
    idx, inv = weld(xyz, rest)
    fr = xyz[:, idx]
    if human_lower_part:
        r = human_lower(fr, rest, inv[tris])
        s = float(np.sqrt((r["err"] ** 2).mean()))
        r["lab"] = r["lab"][inv]
        return r, [sh[1].shape[1] for sh in surfs], tags, s
    best = None
    for seed in range(3):
        r = tree(fr, rest, inv[tris], k, seed)
        s = float(np.sqrt((r["err"] ** 2).mean()))
        if best is None or s < best[0]:
            best = (s, r)
    s, r = best
    r["frames"], r["rest"] = fr, rest
    r["lab_w"] = r["lab"]
    r["lab"] = r["lab"][inv]
    return r, [sh[1].shape[1] for sh in surfs], tags, s


def to_space(p, o, ax):
    return np.asarray(p) @ ax + o


def main(pk3, out_path):
    z = zipfile.ZipFile(pk3)
    out = {}
    for d, parts in MODELS.items():
        if f"models/players/{d}/animation.cfg" not in z.namelist():
            continue
        an = anim_frames(z, d)
        bones, labels, rest_of, report = [], {}, {}, []
        if "nonseg" in parts:
            rest = an.get("NSPA_STAND", [0])[0]
            r, sizes, _, s = part_rig(z, f"models/players/{d}/nonseg.md3", rest, parts["nonseg"])
            rest_of["nonseg"] = rest
            for j in range(r["k"]):
                bones.append(dict(name=f"b{j}", parent=r["parent"][j], part="nonseg", joint=[round(float(v), 3) for v in r["joints"][j]], limit=round(r["limits"][j], 1)))
            labels["nonseg"] = np.split(r["lab"], np.cumsum(sizes)[:-1])
            report.append(f"nonseg {r['k']} bones rms {s:.2f}")
        else:
            lrest = an["LEGS_IDLE"][0]
            urest = an["TORSO_STAND"][0]
            rest_of.update(lower=lrest, upper=urest, head=0)
            lr, lsz, ltags, ls = part_rig(z, f"models/players/{d}/lower.md3", lrest, parts["lower"], human_lower_part=True)
            ur, usz, utags, us = part_rig(z, f"models/players/{d}/upper.md3", urest, parts["upper"])
            ur["lab_rest"] = ur["lab"]
            ur["lab"] = ur["lab_w"]
            ukinds = human_upper_kinds(ur, ur["frames"], urest)
            ur["lab"] = ur["lab_rest"]
            hr, hsz, _, _ = part_rig(z, f"models/players/{d}/head.md3", 0, 1)
            to, tax = ltags[lrest]["tag_torso"]
            ho, hax = utags[urest]["tag_head"]
            lnames = ["pelvis", "thigh_l", "shin_l", "foot_l", "thigh_r", "shin_r", "foot_r"]
            for j in range(lr["k"]):
                bones.append(dict(name=lnames[j], parent=lr["parent"][j], part="lower", joint=[round(float(v), 3) for v in lr["joints"][j]], limit=round(lr["limits"][j], 1), kind=lr["kinds"][j]))
            ubase = len(bones)
            # the upper root hangs on the lower bone nearest tag_torso
            for j in range(ur["k"]):
                par = ur["parent"][j]
                jp = to_space(ur["joints"][j], to, tax)
                limit = ur["limits"][j]
                kind = ukinds[j]
                if par < 0:
                    jp = to  # the waist: tag_torso
                    limit = 45.0
                    kind = ("cone",) + WAIST
                elif kind[0] == "hinge":
                    # the hinge in the lower part's space
                    kind = ("hinge", kind[1] @ tax, kind[2], kind[3], to_space(kind[4], to, tax))
                bones.append(dict(name=f"u{j}", parent=(ubase + par) if par >= 0 else None, part="upper", joint=[round(float(v), 3) for v in jp], limit=round(limit, 1), kind=kind))
            hbase = len(bones)
            hp = to_space(ho, to, tax)
            bones.append(dict(name="head", parent=None, part="head", joint=[round(float(v), 3) for v in hp], limit=50.0, kind=("cone",) + NECK))
            labels["lower"] = np.split(lr["lab"], np.cumsum(lsz)[:-1])
            labels["upper"] = [l + ubase for l in np.split(ur["lab"], np.cumsum(usz)[:-1])]
            labels["head"] = [l * 0 + hbase for l in np.split(hr["lab"], np.cumsum(hsz)[:-1])]
            # the upper root's parent: the lower bone whose vertices are nearest the waist
            nf, ltg, lsurfs = parse(z, f"models/players/{d}/lower.md3")
            lxyz = np.concatenate([s[1][lrest] for s in lsurfs])
            near = np.argsort(np.linalg.norm(lxyz - to, axis=1))[:12]
            waist = int(np.bincount(lr["lab"][near]).argmax())
            nf, utg, usurfs = parse(z, f"models/players/{d}/upper.md3")
            uxyz = to_space(np.concatenate([s[1][urest] for s in usurfs]), to, tax)
            near = np.argsort(np.linalg.norm(uxyz - hp, axis=1))[:12]
            neck = ubase + int(np.bincount(ur["lab"][near]).argmax())
            for bo in bones[ubase:hbase]:
                if bo["parent"] is None:
                    bo["parent"] = waist
            bones[hbase]["parent"] = neck
            for bo in bones[:ubase]:
                if bo["parent"] == -1:
                    bo["parent"] = None
            report.append(f"lower {lr['k']} rms {ls:.2f}, upper {ur['k']} rms {us:.2f}, head 1")
        for bo in bones:
            if bo["parent"] == -1:
                bo["parent"] = None
        out[d] = dict(rest=rest_of, bones=bones, labels={p: [l.astype(int).tolist() for l in ls] for p, ls in labels.items()})
        print(f"{d}: {len(bones)} bones; " + "; ".join(report), flush=True)
    with open(out_path, "w") as f:
        f.write("# rig_md3.py: ragdoll skeletons recovered from the models' own animation (not model data)\n")
        f.write("# model NAME / rest PART FRAME / bone NAME PARENT|- PART X Y Z LIMIT / labels PART SURFACE BONE... / end\n")
        for d, m in out.items():
            f.write(f"model {d}\n")
            for part, fr in m["rest"].items():
                f.write(f"rest {part} {fr}\n")
            for bo in m["bones"]:
                par = "-" if bo["parent"] is None else str(bo["parent"])
                kd = bo.get("kind")
                extra = ""
                if kd and kd[0] == "cone":
                    extra = f" cone {kd[1]:g} {kd[2]:g}"
                elif kd and kd[0] == "hinge":
                    ax, tp = kd[1], kd[4]
                    extra = f" hinge {ax[0]:.4f} {ax[1]:.4f} {ax[2]:.4f} {kd[2]:.1f} {kd[3]:.1f} {tp[0]:.3f} {tp[1]:.3f} {tp[2]:.3f}"
                f.write(f"bone {bo['name']} {par} {bo['part']} {bo['joint'][0]} {bo['joint'][1]} {bo['joint'][2]} {bo['limit']}{extra}\n")
            for part, surfs in m["labels"].items():
                for si, ls in enumerate(surfs):
                    f.write(f"labels {part} {si} " + " ".join(map(str, ls)) + "\n")
            f.write("end\n")
    print(f"SUMMARY rig_md3 status=ok models={len(out)} detail={out_path}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
