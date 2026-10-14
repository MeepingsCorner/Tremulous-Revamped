#!/usr/bin/env bash
# package_windows.sh - Windows release folder (and zip) that starts without a SmartScreen
# "unknown publisher" warning: Godot's own signed 4.7.2 engine exe, renamed
# Tremulous.exe, plus our Tremulous.pck beside it. Godot's export templates are
# unsigned; the engine exe is signed by Godot's publisher, and the signature
# survives the rename. Godot loads <exe name>.pck on start, so no flags needed.
#
# usage: tools/package_windows.sh [--dll FILE] [--project DIR] [--sfx-hd DIR] [--zip FILE]
#                                 <Godot_v4.7.2-stable_win64.exe.zip> <godot-4.7.2-editor> <outdir>
#   zip: the official download; its SHA512 is checked against godot-builds' SHA512-SUMS.txt
#        (set GODOT_WIN_SHA512 to the value listed there for that file)
#   godot-4.7.2-editor: any 4.7.2 editor binary, used for --export-pack only
#   outdir: the release folder to fill (must not exist yet, or be empty)
#   --dll FILE     the trem_godot.dll to ship (default: godot/bin/trem_godot.dll as committed)
#   --project DIR  the Godot project to export Tremulous.pck from (default: godot/, imported
#                  first if it never was)
#   --sfx-hd DIR   ship this sfx_hd/ folder (the HD sounds), as upstream's release zip does
#   --zip FILE     also write the release zip FILE (one top folder named like outdir; sorted,
#                  fixed time SOURCE_DATE_EPOCH, deflate 9) and add it to SHA256SUMS.txt beside it
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
dll=""; proj=$root/godot; sfx=""; zipout=""
while [ $# -gt 0 ]; do
  case $1 in
    --dll) dll=$(readlink -f "$2"); shift;;
    --project) proj=$(cd "$2" && pwd); shift;;
    --sfx-hd) sfx=$(cd "$2" && pwd); shift;;
    --zip) zipout=$(readlink -f "$2"); shift;;
    -h|--help) sed -n '2,19p' "$0"; exit 0;;
    -*) echo "unknown option $1" >&2; exit 2;;
    *) break;;
  esac; shift
done
[ $# -eq 3 ] || { sed -n '8,9p' "$0" >&2; exit 2; }
zip=$1; godot=$2; out=$3
die() { echo "package_windows: $*" >&2; exit 1; }
: "${GODOT_WIN_SHA512:?set GODOT_WIN_SHA512 to the Godot_v4.7.2-stable_win64.exe.zip line of SHA512-SUMS.txt}"
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct 2>/dev/null || echo 1700000000)}
echo "$GODOT_WIN_SHA512  $zip" | sha512sum -c -
mkdir -p "$out"; [ -z "$(ls -A "$out")" ] || die "$out is not empty"
out=$(cd "$out" && pwd)
blob() { git -C "$root" show "HEAD:$1" > "$2" 2>/dev/null || cp "$root/$1" "$2"; }   # committed file, not a local build
unzip -p "$zip" Godot_v4.7.2-stable_win64.exe > "$out/Tremulous.exe"
if [ ! -f "$proj/.godot/extension_list.cfg" ]; then   # first import of a godot-rust project may abort at exit
  "$godot" --headless --import --path "$proj" >/dev/null 2>&1 || true
  "$godot" --headless --import --path "$proj" >/dev/null 2>&1 || true
fi
log=$(dirname "$out")/export-pack-windows.log
"$godot" --headless --path "$proj" --export-pack "Windows Desktop" "$out/Tremulous.pck" > "$log" 2>&1 \
  || { tail -20 "$log"; die "export-pack failed"; }
[ -s "$out/Tremulous.pck" ] || die "no Tremulous.pck exported (see $log)"
if [ -n "$dll" ]; then cp "$dll" "$out/trem_godot.dll"; else blob godot/bin/trem_godot.dll "$out/trem_godot.dll"; fi
for f in trem_audio fmod phonon phonon_fmod; do blob "godot/bin/$f.dll" "$out/$f.dll"; done
cp -a "$root/godot/base" "$out/base"
[ -z "$sfx" ] || cp -a "$sfx" "$out/sfx_hd"
sed 's/$/\r/' "$root/tools/README-WINDOWS.txt.in" > "$out/README.txt"   # CRLF, as upstream ships it
find "$out" -type f -exec chmod 644 {} +; chmod 755 "$out/Tremulous.exe"
# Optional signer check. osslsigncode may report "unable to get local issuer
# certificate": the signature carries no copy of the Certum code-signing
# intermediate (Windows fetches it itself); it verifies once that CA cert is in -CAfile.
command -v osslsigncode >/dev/null && osslsigncode verify -in "$out/Tremulous.exe" | grep -m1 "Subject: CN=Prehensile" || true
[ -n "$zipout" ] || exit 0
python3 -I - "$zipout" "$out" <<'PY'
import os, sys, zipfile, time
dst, src = sys.argv[1:3]
top = os.path.basename(src)
t = time.gmtime(int(os.environ["SOURCE_DATE_EPOCH"]))[:6]
files = sorted(os.path.relpath(os.path.join(r, f), src) for r, _, fs in os.walk(src) for f in fs)
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for f in files:
        p = os.path.join(src, f)
        zi = zipfile.ZipInfo(top + "/" + f.replace(os.sep, "/"), t)
        zi.create_system = 3
        zi.compress_type = zipfile.ZIP_DEFLATED
        zi.external_attr = ((0o755 if os.access(p, os.X_OK) else 0o644) | 0o100000) << 16
        with open(p, "rb") as fh: z.writestr(zi, fh.read(), compresslevel=9)
print(dst, len(files), "files")
PY
cd "$(dirname "$zipout")"; z=$(basename "$zipout")
{ [ -f SHA256SUMS.txt ] && grep -v "  $z\$" SHA256SUMS.txt || true; sha256sum "$z"; } > SHA256SUMS.new
mv SHA256SUMS.new SHA256SUMS.txt; grep "  $z\$" SHA256SUMS.txt
