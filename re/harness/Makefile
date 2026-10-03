# Reference harness: builds the ORIGINAL Tremulous shared game code natively.
# Deterministic flags: no FMA contraction, no fast-math (D9).
S      := ../source/src
CC     ?= gcc
CFLAGS := -O1 -g -ffp-contract=off -fno-fast-math -DARCH_STRING='"x86_64"' -DGAME \
          -I$(S)/qcommon -I$(S)/game -w
ORIG   := $(S)/game/bg_misc.c $(S)/game/bg_pmove.c $(S)/game/bg_slidemove.c \
          $(S)/qcommon/q_math.c $(S)/qcommon/q_shared.c
CM     := $(S)/qcommon/cm_trace.c $(S)/qcommon/cm_test.c

all: extract consts pmove_ref cm_ref net_ref curve_ref pak_ref msg69_ref
all: extract consts pmove_ref cm_ref net_ref curve_ref pak_ref v11

extract: extract.c shims.c $(ORIG)
	$(CC) $(CFLAGS) -o $@ $^ -lm

consts: consts.c
	$(CC) $(CFLAGS) -o $@ $^ -lm

consts.c: gen_consts.py $(S)/game/tremulous.h $(S)/game/bg_public.h
	python3 gen_consts.py $(S)/qcommon/q_shared.h $(S)/game/bg_public.h $(S)/game/bg_local.h $(S)/game/tremulous.h > $@

pmove_ref: pmove_ref.c cm_world.c shims.c $(ORIG) $(CM)
	$(CC) $(CFLAGS) -DBSPC_STUBS -o $@ $^ -lm

# Real-BSP collision oracle (cm_load + cm_trace + cm_test + cm_patch + cm_polylib)
CMFULL := $(S)/qcommon/cm_load.c $(S)/qcommon/cm_trace.c $(S)/qcommon/cm_test.c \
          $(S)/qcommon/cm_patch.c $(S)/qcommon/cm_polylib.c $(S)/qcommon/q_math.c $(S)/qcommon/q_shared.c
cm_ref: cm_ref.c $(CMFULL)
	$(CC) $(CFLAGS) -o $@ $^ -lm

# Network oracle (msg.c + huffman.c + net_chan.c): tests/net.rs, re/traces/net.sha256
NET := $(S)/qcommon/msg.c $(S)/qcommon/huffman.c $(S)/qcommon/net_chan.c $(S)/qcommon/q_shared.c $(S)/qcommon/q_math.c
net_ref: net_ref.c $(NET)
	$(CC) $(CFLAGS) -o $@ $^ -lm

# Protocol 69 oracle: Tremulous 1.1.0's msg.c + huffman.c + net_chan.c
# (re/source11) and cl_net_chan.c's coders: tests/net_v11.rs,
# re/traces/msg69.sha256 (dec040)
S11    := ../source11/src
NET11  := $(S11)/qcommon/msg.c $(S11)/qcommon/huffman.c $(S11)/qcommon/net_chan.c $(S11)/qcommon/q_shared.c $(S11)/qcommon/q_math.c
msg69_ref: msg69_ref.c $(NET11)
	$(CC) -O1 -g -ffp-contract=off -fno-fast-math -DARCH_STRING='"x86_64"' -I$(S11)/qcommon -w -o $@ $^ -lm

# Pure-pak oracle (unzip.c + ioapi.c + md4.c + zlib, FS_LoadZipFile's checksum
# loop): tests/pure.rs, re/traces/pak.sha256 (dec037)
PAK := $(S)/qcommon/unzip.c $(S)/qcommon/ioapi.c $(S)/qcommon/md4.c $(S)/qcommon/q_shared.c $(S)/qcommon/q_math.c $(wildcard $(S)/zlib/*.c)
pak_ref: pak_ref.c $(PAK)
	$(CC) $(CFLAGS) -I$(S)/zlib -o $@ $^ -lm

# Patch subdivision oracle: the original renderergl1/tr_curve.c, copied next
# to curve/tr_local.h (a stub with only the types it uses)
curve/tr_curve.c: $(S)/renderergl1/tr_curve.c
	cp $< $@
curve_ref: curve_ref.c curve/tr_curve.c $(S)/qcommon/q_math.c $(S)/qcommon/q_shared.c
	$(CC) $(CFLAGS) -Icurve -o $@ $^ -lm

# Tremulous 1.1.0 (protocol 69) oracles (dec041): the 1.1.0 bg code built
# from re/source11 with its own headers; shims, the sheet world and the
# collision (shared with the port, unchanged) keep the legacy headers, so
# each group is compiled on its own and linked together.
S11      := ../source11/src
CFLAGS11 := -O1 -g -ffp-contract=off -fno-fast-math -DGAME -I$(S11)/qcommon -I$(S11)/game -w
BG11     := $(S11)/game/bg_misc.c $(S11)/game/bg_pmove.c $(S11)/game/bg_slidemove.c \
            $(S11)/qcommon/q_math.c $(S11)/qcommon/q_shared.c
v11: consts11 v11_ref pmove11_ref
consts11.c: gen_consts11.py $(S11)/game/tremulous.h $(S11)/game/bg_public.h
	python3 gen_consts11.py $(S11)/qcommon/q_shared.h $(S11)/game/bg_public.h $(S11)/game/bg_local.h $(S11)/game/tremulous.h > $@
consts11: consts11.c
	$(CC) $(CFLAGS11) -o $@ $^ -lm
v11o/legacy.stamp: shims.c cm_world.c $(CM)
	mkdir -p v11o/legacy && cd v11o/legacy && $(CC) $(CFLAGS) -I$(abspath $(S)/qcommon) -I$(abspath $(S)/game) -DBSPC_STUBS -c $(abspath shims.c cm_world.c $(CM))
	touch $@
v11o/bg11.stamp: $(BG11)
	mkdir -p v11o/bg11 && cd v11o/bg11 && $(CC) $(CFLAGS11) -I$(abspath $(S11)/qcommon) -I$(abspath $(S11)/game) -c $(abspath $(BG11))
	touch $@
v11_ref: v11_ref.c v11o/bg11.stamp v11o/legacy.stamp
	$(CC) $(CFLAGS11) -o $@ v11_ref.c v11o/bg11/*.o v11o/legacy/shims.o -lm
pmove11_ref: pmove11_ref.c v11o/bg11.stamp v11o/legacy.stamp
	$(CC) $(CFLAGS11) -o $@ pmove11_ref.c v11o/bg11/*.o v11o/legacy/shims.o v11o/legacy/cm_world.o v11o/legacy/cm_trace.o v11o/legacy/cm_test.o -lm

clean:
	rm -f extract consts consts.c pmove_ref cm_ref net_ref pak_ref curve_ref msg69_ref curve/tr_curve.c
.PHONY: all clean
	rm -f extract consts consts.c pmove_ref cm_ref net_ref pak_ref curve_ref curve/tr_curve.c consts11 consts11.c v11_ref pmove11_ref
	rm -rf v11o
.PHONY: all clean v11
