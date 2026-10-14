#!/bin/sh
# zig cc as a C compiler / linker for glibc $GLIBC_BASELINE (default 2.28).
# Invoked as zig-cc, zig-c++ or zig-ar (symlinks or copies named so). Drops the
# --target=<rust triple> argument the cc crate adds, since zig takes its own.
ZIG=${ZIG:?set ZIG to the zig binary}
base=${GLIBC_BASELINE:-2.28}
case $(basename "$0") in
  zig-ar) exec "$ZIG" ar "$@";;
  zig-c++) mode=c++;;
  *) mode=cc;;
esac
args=
for a in "$@"; do
  case $a in --target=*) continue;; esac
  args="$args
$a"
done
IFS='
'
# shellcheck disable=SC2086
set -- $args
exec "$ZIG" $mode -target "x86_64-linux-gnu.$base" "$@"
