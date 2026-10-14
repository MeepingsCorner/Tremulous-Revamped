#!/usr/bin/env bash
# test-matrix.sh ZIP [OUTDIR]  - headless smoke test of the Linux zip in several distro containers.
# One server container (ubuntu:24.04) runs the game as a dedicated server on a docker network with no outside
# access (--internal); one client container per distro starts the game headless, loads base/, connects, and
# the run counts as PASS when the server logs ClientBegin for that client (the client reached Active).
# Images pulled by this script are removed at the end; images that were already there are kept.
# Needs docker, unzip, python3. Writes OUTDIR/results.tsv and one log per image.
set -uo pipefail
ZIP=$(readlink -f "${1:?usage: test-matrix.sh ZIP [OUTDIR]}"); OUT=$(mkdir -p "${2:-matrix-out}" && cd "${2:-matrix-out}" && pwd)
IMAGES=${IMAGES:-"ubuntu:18.04 debian:10 debian:11 debian:12 ubuntu:22.04 ubuntu:24.04 ubuntu:26.04 fedora:latest archlinux:latest"}
SRVIMG=ubuntu:24.04; NET=trem-lintest-$$; PORT=30790; WAIT=${WAIT:-60}
G=$OUT/game; rm -rf "$G"; mkdir -p "$G"; unzip -q "$ZIP" -d "$G"; G=$G/Tremulous-Revamped
EXE=Tremulous.x86_64; [ -x "$G/$EXE" ] || { echo "no $EXE in zip"; exit 2; }
had=$(docker images --format '{{.Repository}}:{{.Tag}}'); pulled=
for i in $IMAGES $SRVIMG; do
  echo "$had" | grep -qx "$i" || { case " $pulled " in *" $i "*) ;; *) pulled="$pulled $i";; esac; }
done
cleanup() { docker rm -f "trem-lt-srv-$$" >/dev/null 2>&1; for c in $(docker ps -aq --filter "label=trem-lt=$$"); do docker rm -f "$c" >/dev/null 2>&1; done
  docker network rm "$NET" >/dev/null 2>&1; for i in $pulled; do docker rmi "$i" >/dev/null 2>&1; done; echo "removed images:$pulled" > "$OUT/cleanup.txt"; }
trap cleanup EXIT
docker network create --internal "$NET" >/dev/null || exit 2
for i in $IMAGES $SRVIMG; do docker image inspect "$i" >/dev/null 2>&1 || docker pull -q "$i" >/dev/null 2>&1 || echo "pull failed: $i" >&2; done
run() { docker run -t "$@"; }
ENVS="-e HOME=/tmp/h -e XDG_DATA_HOME=/tmp/h"
docker run -dt --rm --name "trem-lt-srv-$$" --label trem-lt=$$ --network "$NET" -v "$G":/game:ro $ENVS "$SRVIMG" \
  /game/$EXE --headless -- +set dedicated 1 +set net_port $PORT +map atcs >/dev/null
for _ in $(seq 40); do docker logs "trem-lt-srv-$$" 2>&1 | grep -q 'Server atcs spawned' && break; sleep 1; done
printf 'image\tglibc\tresult\tnote\n' > "$OUT/results.tsv"
for img in $IMAGES; do
  tag=$(echo "$img" | tr ':/' '__'); c=trem-lt-cl-$$-$tag
  if ! docker image inspect "$img" >/dev/null 2>&1; then printf '%s\t-\tSKIP\tcould not pull\n' "$img" >> "$OUT/results.tsv"; continue; fi
  gl=$(docker run --rm --entrypoint sh "$img" -c 'ldd --version 2>&1 | head -1' 2>/dev/null | grep -o '[0-9]*\.[0-9]*$')
  docker run -dt --name "$c" --label trem-lt=$$ --network "$NET" -v "$G":/game:ro $ENVS "$img" \
    /game/$EXE --headless -- +set name "LT_$tag" +connect "trem-lt-srv-$$:$PORT" >/dev/null
  ip=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$NET\").IPAddress}}" "$c")
  res=FAIL; note=
  for _ in $(seq "$WAIT"); do
    s=$(docker logs "trem-lt-srv-$$" 2>&1 | tr -d '\r')
    slot=$(echo "$s" | grep "ClientConnect: .*\[$ip\]" | tail -1 | sed 's/.*ClientConnect: \([0-9]*\) .*/\1/')
    if [ -n "$slot" ] && echo "$s" | sed -n "/ClientConnect: $slot \[$ip\]/,\$p" | grep -q "ClientBegin: $slot"; then res=PASS; break; fi
    [ "$(docker inspect -f '{{.State.Running}}' "$c")" = false ] && break
    sleep 1
  done
  docker logs "$c" 2>&1 | tr -d '\r' > "$OUT/client-$tag.log"
  if [ $res = PASS ]; then
    sleep 3; [ "$(docker inspect -f '{{.State.Running}}' "$c")" = true ] || { res=FAIL; note="exited after join"; }
  else
    note=$(grep -m1 -E 'GLIBC|error|ERROR|not found|No such' "$OUT/client-$tag.log" | cut -c1-110)
  fi
  [ $res = PASS ] && note="joined as client $slot, still running after 3 s"
  printf '%s\t%s\t%s\t%s\n' "$img" "${gl:--}" "$res" "$note" >> "$OUT/results.tsv"
  docker rm -f "$c" >/dev/null 2>&1
done
docker logs "trem-lt-srv-$$" 2>&1 | tr -d '\r' > "$OUT/server.log"
column -t -s$'\t' "$OUT/results.tsv"
