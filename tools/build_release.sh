#!/usr/bin/env bash
# build_release.sh - one command from an upstream checkout to the release zip(s).
#
#   tools/build_release.sh [--src DIR] [--out DIR] [--godot BIN] [--sfx-hd DIR]
#                          [--combined] [--windows-dll FILE | --windows-mingw] [--glibc 2.28]
#
# Builds libtrem_godot.so against an OLD glibc (default 2.28) with zig cc as C
# compiler and linker (no containers, no root), exports the game with Godot's
# "Linux" preset and writes, byte-reproducibly:
#   Tremulous-Revamped-linux-x86_64.zip         Linux only
#   Tremulous-Revamped-win64-linux-x86_64.zip   (with --combined) Windows + Linux, one base/
# plus SHA256SUMS.txt. The layout mirrors upstream's Windows zip: one folder
# Tremulous-Revamped/ holding the executable, Tremulous.pck, the GDExtension and
# audio libraries next to it, base/, optional sfx_hd/, a README.
#
# Needs: cargo (rust >= the workspace rust-version), python3, git, a Godot
# EDITOR and the matching export templates (same version as upstream's release
# uses: see EXPECT_GODOT below), pip3 (only to fetch zig when ZIG is unset).
# --combined also needs the "Windows Desktop" template. Its Windows library is
# the trem_godot.dll committed in godot/bin (upstream's own MSVC build, so the
# Windows half equals upstream's release zip); --windows-dll FILE uses another,
# --windows-mingw cross-builds it (needs x86_64-w64-mingw32-gcc and the
# x86_64-pc-windows-gnu rust target).
#
# Environment: GODOT (editor binary), ZIG (zig binary), SOURCE_DATE_EPOCH.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SRC=$(cd "$HERE/.." && pwd)
OUT=""; GODOT=${GODOT:-godot}; SFX=""; COMBINED=0; WINDLL=""; MINGW=0; GLIBC=2.28
EXPECT_GODOT="4.7.2.stable"   # upstream's v1.0 Windows exe is the 4.7.2 template
while [ $# -gt 0 ]; do
  case $1 in
    --src) SRC=$(cd "$2" && pwd); shift;;
    --out) OUT=$2; shift;;
    --godot) GODOT=$2; shift;;
    --sfx-hd) SFX=$(cd "$2" && pwd); shift;;
    --combined) COMBINED=1;;
    --windows-dll) WINDLL=$(readlink -f "$2"); shift;;
    --windows-mingw) MINGW=1;;
    --glibc) GLIBC=$2; shift;;
    -h|--help) sed -n '2,22p' "$0"; exit 0;;
    *) echo "unknown option $1" >&2; exit 2;;
  esac; shift
done
die() { echo "build_release: $*" >&2; exit 1; }
[ -f "$SRC/godot/project.godot" ] || die "$SRC is not an upstream checkout (no godot/project.godot); use --src"
OUT=$(mkdir -p "${OUT:-$SRC/target/release-zip}" && cd "${OUT:-$SRC/target/release-zip}" && pwd)
command -v "$GODOT" >/dev/null || die "Godot editor not found (set GODOT or --godot)"
ver=$("$GODOT" --version 2>/dev/null | head -1 | cut -d. -f1-4)
[ "${ver%.official*}" = "$EXPECT_GODOT" ] || echo "WARNING: Godot is $ver, upstream's release uses $EXPECT_GODOT (zip will differ from a parity build)" >&2
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$SRC" log -1 --format=%ct 2>/dev/null || echo 1700000000)}
W=$OUT/work; rm -rf "$W"; mkdir -p "$W"

# ---- 1. zig (old-glibc C toolchain) -------------------------------------------
if [ -z "${ZIG:-}" ]; then
  if command -v zig >/dev/null; then ZIG=$(command -v zig)
  else
    echo "== fetching zig with pip (PyPI package ziglang)"
    pip3 install --quiet --break-system-packages --target "$OUT/zig-pkg" ziglang >&2
    ZIG=$OUT/zig-pkg/ziglang/zig
  fi
fi
export ZIG GLIBC_BASELINE=$GLIBC
mkdir -p "$W/bin"
for n in zig-cc zig-c++ zig-ar; do cp "$HERE/zig-cc-wrapper.sh" "$W/bin/$n"; done

# ---- 2. libtrem_godot.so against glibc $GLIBC -----------------------------------
echo "== cargo build (glibc $GLIBC)"
TGT=$OUT/target-linux
# remap source, cargo registry and target dir (generated files) so no local path ends up in the binaries;
# rustc gives the LAST matching mapping priority, so the longest prefixes go last
RF="--remap-path-prefix=$SRC=/src"$'\x1f'"--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo"$'\x1f'"--remap-path-prefix=$OUT=/out"
( cd "$SRC"
  export CC_x86_64_unknown_linux_gnu=$W/bin/zig-cc CXX_x86_64_unknown_linux_gnu=$W/bin/zig-c++ \
         AR_x86_64_unknown_linux_gnu=$W/bin/zig-ar CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$W/bin/zig-cc \
         CARGO_ENCODED_RUSTFLAGS=$RF CARGO_INCREMENTAL=0
  cargo build --release --target x86_64-unknown-linux-gnu -p trem_godot --target-dir "$TGT" )
SO=$TGT/x86_64-unknown-linux-gnu/release/libtrem_godot.so
need=$(objdump -T "$SO" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1)
echo "libtrem_godot.so needs $need ; NEEDED: $(readelf -d "$SO" | grep -o 'Shared library: \[[^]]*\]' | sed 's/.*\[//;s/\]//' | tr '\n' ' ')"
[ "$need" = "GLIBC_$GLIBC" ] || [ "$(printf '%s\n' "GLIBC_$GLIBC" "$need" | sort -V | tail -1)" = "GLIBC_$GLIBC" ] || die "library needs $need, wanted <= GLIBC_$GLIBC"

# ---- 3. staging project (the repo tree stays untouched) -----------------------
P=$W/project; mkdir -p "$P"
( cd "$SRC/godot" && tar cf - --exclude=./.godot --exclude=./shots . ) | tar xf - -C "$P"
grep -q 'name="Linux"' "$P/export_presets.cfg" || cat "$HERE/../presets/linux-preset.cfg" >> "$P/export_presets.cfg"
mkdir -p "$P/bin"; cp "$SO" "$P/bin/libtrem_godot.so"
TPL=${GODOT_TEMPLATES:-${XDG_DATA_HOME:-$HOME/.local/share}/godot/export_templates}
[ -d "$TPL/$EXPECT_GODOT" ] || echo "WARNING: no export templates in $TPL/$EXPECT_GODOT (set GODOT_TEMPLATES)" >&2
export XDG_DATA_HOME=$W/xdg XDG_CONFIG_HOME=$W/xdg-config   # keep the user's editor settings out
mkdir -p "$XDG_DATA_HOME/godot"; ln -sfn "$TPL" "$XDG_DATA_HOME/godot/export_templates"
"$GODOT" --headless --import --path "$P" >/dev/null 2>&1 || true    # first import of a godot-rust project may abort at exit
"$GODOT" --headless --import --path "$P" >/dev/null 2>&1 || true
[ -f "$P/.godot/extension_list.cfg" ] || die "import did not register the GDExtension"

# ---- 4. exports ----------------------------------------------------------------
LX=$W/export-linux; mkdir -p "$LX"
echo "== export Linux"; "$GODOT" --headless --path "$P" --export-release Linux "$LX/Tremulous.x86_64" >"$W/export-linux.log" 2>&1
[ -x "$LX/Tremulous.x86_64" ] && [ -f "$LX/Tremulous.pck" ] || { tail -20 "$W/export-linux.log"; die "Linux export failed"; }
if [ $COMBINED = 1 ]; then
  if [ -z "$WINDLL" ] && [ $MINGW = 0 ]; then WINDLL=$W/trem_godot.dll
    # the committed blob, not the working-tree file (local builds overwrite that one)
    git -C "$SRC" show HEAD:godot/bin/trem_godot.dll > "$WINDLL" 2>/dev/null || cp "$SRC/godot/bin/trem_godot.dll" "$WINDLL"
    echo "== Windows library: godot/bin/trem_godot.dll as committed"; fi
  if [ -z "$WINDLL" ]; then
    echo "== cargo build (windows-gnu, mingw)"
    ( cd "$SRC"
      export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
             CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
             CFLAGS_x86_64_pc_windows_gnu="-ffile-prefix-map=$SRC=/src" CARGO_INCREMENTAL=0 \
             CARGO_ENCODED_RUSTFLAGS="$RF"$'\x1f-Clink-arg=-Wl,--no-insert-timestamp'
      cargo build --release --target x86_64-pc-windows-gnu -p trem_godot --target-dir "$OUT/target-win" )
    WINDLL=$OUT/target-win/x86_64-pc-windows-gnu/release/trem_godot.dll
  fi
  [ -f "$WINDLL" ] || die "no Windows library at $WINDLL"
  echo "windows library sha256 $(sha256sum "$WINDLL" | cut -d' ' -f1)"
  cp "$WINDLL" "$P/bin/trem_godot.dll"
  WX=$W/export-win; mkdir -p "$WX"
  echo "== export Windows Desktop"; "$GODOT" --headless --path "$P" --export-release "Windows Desktop" "$WX/Tremulous.exe" >"$W/export-win.log" 2>&1
  [ -f "$WX/Tremulous.exe" ] || { tail -20 "$W/export-win.log"; die "Windows export failed"; }
  cmp -s "$WX/Tremulous.pck" "$LX/Tremulous.pck" || die "the two platforms exported different .pck files: they cannot share one"
fi

# ---- 5. assemble + zip -----------------------------------------------------------
T=Tremulous-Revamped
engine_glibc=$(objdump -T "$LX/Tremulous.x86_64" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1 | sed 's/GLIBC_//')
REQ=$(printf '%s\n' "${need#GLIBC_}" "$engine_glibc" | sort -V | tail -1)
fill() { sed -e "s/@GLIBC@/${need#GLIBC_}/g" -e "s/@ENGINE_GLIBC@/$engine_glibc/g" -e "s/@REQ@/$REQ/g" -e 's/@REPO_NOTE@/the release notes./' "$1"; }
python_zip() {  # python_zip OUT.zip STAGEDIR  (sorted, fixed time, unix modes, deflate 9)
  python3 -I - "$1" "$2" "$T" <<'PY'
import os, sys, zipfile, time
dst, src, top = sys.argv[1:4]
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
}
common() {  # base/, optional sfx_hd/ into $1
  cp -a "$SRC/godot/base" "$1/base"; find "$1/base" -type f -exec chmod 644 {} +
  if [ -n "$SFX" ]; then cp -a "$SFX" "$1/sfx_hd"; fi
}
# Linux only
S=$W/stage-linux/$T; mkdir -p "$S"
install -m755 "$LX/Tremulous.x86_64" "$S/"; install -m644 "$LX/Tremulous.pck" "$S/"; install -m755 "$SO" "$S/libtrem_godot.so"
fill "$HERE/README-LINUX.txt.in" > "$S/README-LINUX.txt"; common "$S"
python_zip "$OUT/Tremulous-Revamped-linux-x86_64.zip" "$S"
if [ $COMBINED = 1 ]; then
  S=$W/stage-both/$T; mkdir -p "$S"
  install -m755 "$LX/Tremulous.x86_64" "$S/"; install -m755 "$WX/Tremulous.exe" "$S/"; install -m644 "$LX/Tremulous.pck" "$S/"
  install -m755 "$SO" "$S/libtrem_godot.so"; install -m644 "$WINDLL" "$S/trem_godot.dll"
  for f in fmod.dll phonon.dll phonon_fmod.dll trem_audio.dll; do install -m644 "$SRC/godot/bin/$f" "$S/$f"; done
  fill "$HERE/README-BOTH.txt.in" > "$S/README.txt"; common "$S"
  python_zip "$OUT/Tremulous-Revamped-win64-linux-x86_64.zip" "$S"
fi
( cd "$OUT" && sha256sum Tremulous-Revamped-*.zip > SHA256SUMS.txt && cat SHA256SUMS.txt && ls -l Tremulous-Revamped-*.zip )
echo "glibc: libtrem_godot.so $need, engine binary GLIBC_$engine_glibc, zip requires glibc $REQ"
