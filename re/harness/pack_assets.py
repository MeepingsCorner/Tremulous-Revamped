#!/usr/bin/env python3
"""Pack tremulous-legacy's own `assets/` tree (re/source/assets: legacy UI
menus and HUDs, buildstat.cfg, particle/trail/shader overrides, weapon and
animation cfgs) into godot/base/data-legacy-assets.pk3.

The original client loads it after data-1.1.0.pk3 so its files override the
1.1.0 ones; Pk3Set sorts paths the same way ("data-1.1.0*" < "data-legacy*"),
so later archives win. Deterministic: sorted entries, fixed timestamps.
"""
import pathlib, zipfile
root = pathlib.Path(__file__).resolve().parents[2]
src = root / "re/source/assets"
out = root / "godot/base/data-legacy-assets.pk3"
files = sorted(p for p in src.rglob("*") if p.is_file())
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for p in files:
        info = zipfile.ZipInfo(p.relative_to(src).as_posix(), date_time=(2010, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, p.read_bytes())
print(f"{out.relative_to(root)}: {len(files)} files")
