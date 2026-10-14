#!/usr/bin/env bash
# merge-check.sh WIN_ZIP LINUX_ZIP [WORKDIR]
# Proves the Linux zip merges cleanly with upstream's Windows zip: extracts them over each other in both
# orders and checks that no file's content changes, lists the overlapping and the Linux-only files, and
# leaves the two merged folders (win-then-linux, linux-then-win) in WORKDIR for further tests.
set -euo pipefail
WIN=$(readlink -f "${1:?usage: merge-check.sh WIN_ZIP LINUX_ZIP [WORKDIR]}"); LIN=$(readlink -f "${2:?}")
W=$(mkdir -p "${3:-merge-test}" && cd "${3:-merge-test}" && pwd); rm -rf "$W"/{win-then-linux,linux-then-win}
mkdir -p "$W/win-then-linux" "$W/linux-then-win"
unzip -q "$WIN" -d "$W/win-then-linux"; unzip -q -o "$LIN" -d "$W/win-then-linux"
unzip -q "$LIN" -d "$W/linux-then-win"; unzip -q -o "$WIN" -d "$W/linux-then-win"
python3 -I - "$WIN" "$LIN" "$W" <<'PY'
import hashlib, os, sys, zipfile
win, lin, w = sys.argv[1:4]
def h(b): return hashlib.sha256(b).hexdigest()
wz, lz = zipfile.ZipFile(win), zipfile.ZipFile(lin)
wf = {n: h(wz.read(n)) for n in wz.namelist() if not n.endswith("/")}
lf = {n: h(lz.read(n)) for n in lz.namelist() if not n.endswith("/")}
tops = {n.split("/")[0] for n in list(wf) + list(lf)}
print("top-level folders:", sorted(tops)); bad = 0
if len(tops) != 1: print("FAIL: different top-level folders"); bad += 1
overlap = sorted(set(wf) & set(lf)); lonly = sorted(set(lf) - set(wf)); wonly = sorted(set(wf) - set(lf))
clash = [n for n in overlap if wf[n] != lf[n]]
print(f"windows zip {len(wf)} files, linux zip {len(lf)} files, overlapping {len(overlap)}, linux-only {len(lonly)}, windows-only {len(wonly)}")
print("overlapping files whose content differs:", clash or "none"); bad += len(clash)
for order in ("win-then-linux", "linux-then-win"):
    d = os.path.join(w, order); got = {}
    for r, _, fs in os.walk(d):
        for f in fs:
            p = os.path.join(r, f); got[os.path.relpath(p, d)] = h(open(p, "rb").read())
    exp = {**wf, **lf}
    diff = [n for n in exp if got.get(n) != exp[n]] + [n for n in got if n not in exp]
    print(f"{order}: {len(got)} files; content differs from the zips: {diff or 'none'}"); bad += len(diff)
    for n in ("Tremulous-Revamped/Tremulous.x86_64", "Tremulous-Revamped/Tremulous.exe"):
        if n in got and n.endswith("86_64") and not os.access(os.path.join(d, n), os.X_OK): print("FAIL: not executable:", n); bad += 1
print("overlap:", *[n for n in overlap if not n.startswith("Tremulous-Revamped/base/") and not n.startswith("Tremulous-Revamped/sfx_hd/")], sep="\n  ")
print(f"  base/: {sum(n.startswith('Tremulous-Revamped/base/') for n in overlap)} files, sfx_hd/: {sum(n.startswith('Tremulous-Revamped/sfx_hd/') for n in overlap)} files")
print("linux-only:", *lonly, sep="\n  ")
print("RESULT:", "MERGES CLEANLY" if not bad else f"PROBLEMS ({bad})"); sys.exit(1 if bad else 0)
PY
