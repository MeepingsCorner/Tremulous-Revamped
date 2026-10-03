#!/usr/bin/env python3
"""Mode A symbol map (MDD §7.3): one row per function definition in the original
source, with ref_src = file:line. Initial status is assigned by subsystem; the
sheet is then the truth and statuses are edited by hand / by row patches.

usage: symbols.py <repo_root>   (writes sheets/re/symbols.tsv only if absent,
                                 otherwise prints rows missing from the sheet)
"""
import os, re, sys

ROOT = sys.argv[1]
SRC = os.path.join(ROOT, 're/source/src')

# subsystem -> (dirs/files, default status, planned rust module)
SUBSYS = [
    ('bg', ['game/bg_misc.c', 'game/bg_pmove.c', 'game/bg_slidemove.c'], 'Todo', 'trem_sim::bg'),
    ('bg_aux', ['game/bg_alloc.c', 'game/bg_voice.c', 'game/bg_lib.c'], 'Dropped', 'NULL'),
    ('qmath', ['qcommon/q_math.c'], 'Todo', 'trem_sim::qmath'),
    ('cm', ['qcommon/cm_trace.c', 'qcommon/cm_test.c', 'qcommon/cm_load.c'], 'Todo', 'trem_sim::cm'),
    ('game', ['game/g_*.c'], 'Todo', 'trem_sim::game'),
    ('cgame', ['cgame/cg_*.c'], 'Replaced', 'trem_godot'),
    ('ui', ['ui/ui_*.c'], 'Replaced', 'godot'),
    # networking (dec033): the protocol, the server and the client engine
    ('net', ['qcommon/huffman.c', 'qcommon/msg.c', 'qcommon/net_chan.c', 'qcommon/net_ip.c', 'qcommon/md5.c'], 'Todo', 'trem_sim::netchan'),
    ('server', ['server/sv_*.c'], 'Todo', 'trem_sim::sv'),
    ('client', ['client/cl_main.c', 'client/cl_parse.c', 'client/cl_input.c', 'client/cl_cgame.c', 'client/cl_net_chan.c'], 'Todo', 'trem_sim::cl'),
]
DROP_GAME = ('g_admin.c', 'g_syscalls.c', 'g_svcmds.c', 'g_maprotation.c', 'g_session.c', 'g_ptr.c',
             'g_namelog.c', 'g_main.c:CheckVote', 'g_cmds.c')

FN = re.compile(r'^(?:static\s+|inline\s+|QDECL\s+)*[A-Za-z_][\w\s\*]*?\b([A-Za-z_]\w*)\s*\(([^;]*)$')


def files(pattern):
    import glob
    return sorted(glob.glob(os.path.join(SRC, pattern)))


def scan(path):
    lines = open(path, errors='replace').read().splitlines()
    for i, l in enumerate(lines):
        if not l or l[0] in ' \t#/{}*' or l.startswith('typedef') or l.rstrip().endswith(';'):
            continue
        m = FN.match(l)
        if not m:
            continue
        name = m.group(1)
        if name in ('if', 'while', 'for', 'switch', 'return', 'sizeof'):
            continue
        # body must open within the next 3 lines
        nxt = ' '.join(lines[i:i + 4])
        if '{' not in nxt or ';' in nxt.split('{')[0]:
            continue
        yield name, i + 1


rows, seen = [], set()
for sub, pats, status, module in SUBSYS:
    for pat in pats:
        for p in files(pat):
            rel = os.path.relpath(p, SRC)
            base = os.path.basename(p)
            for name, ln in scan(p):
                sid = re.sub(r'[^a-z0-9_]', '_', name.lower())
                if sid in seen:
                    sid = f'{sid}__{os.path.splitext(base)[0]}'
                if sid in seen:
                    continue
                seen.add(sid)
                st = status
                if sub == 'game' and base in DROP_GAME:
                    st = 'Dropped'
                rows.append([sid, 'fn', sub, f'{rel}:{ln}', module, st])

rows.sort(key=lambda r: (r[2], r[3].split(':')[0], int(r[3].split(':')[1])))
out = os.path.join(ROOT, 'sheets/re/symbols.tsv')
if os.path.exists(out):
    have = {l.split('\t')[0] for l in open(out).read().splitlines() if l and not l.startswith('#')}
    missing = [r for r in rows if r[0] not in have]
    print(f'{len(rows)} symbols in source, {len(missing)} missing from sheet')
    for r in missing:
        print('\t'.join(r))
    sys.exit(0)
os.makedirs(os.path.dirname(out), exist_ok=True)
with open(out, 'w', newline='\n') as f:
    f.write('# sheet: symbols\n# version: 1\n# generator: none\n# owned_by: extractor\n# budget: 48000\n'
            '# notes: Mode A symbol map, MDD 7.3. ordered by subsystem, file, line\n')
    f.write('id\tkind\tsubsystem\tref_src\trust_module\tstatus\n')
    for r in rows:
        f.write('\t'.join(r) + '\n')
print(len(rows), 'symbols written')
