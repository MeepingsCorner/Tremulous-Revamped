#!/usr/bin/env python3
"""Tremulous 1.1.0 sheets (dec041) from the ORIGINAL 1.1.0 code's dumps:
re/traces/v11/tables.tsv (re/harness/v11_ref: bg_misc.c's BG_Find* with
data-1.1.0.pk3's overrides) and re/traces/v11/consts.tsv (re/harness/consts11:
the 1.1.0 headers' defines and enums, evaluated by the compiler).

Writes sheets/domain/{classes11,weapons11,upgrades11,buildables11,
constants11}.tsv, the 1.1 enum domains of sheets/01-enums.tsv (Ability11,
Event11, MeansOfDeath11, EntityType11, Stat11, Pers11, Menu11, PmType11),
their schema rows and groups. Idempotent: re-running replaces its own rows.

usage: synth_v11.py [repo_root]"""
import os, re, sys
import numpy as np

ROOT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), '../..')
S = os.path.join(ROOT, 'sheets')
T = os.path.join(ROOT, 're/traces/v11')
BG11 = os.path.join(ROOT, 're/source11/src/game/bg_misc.c')


def f32(x):
    s = str(np.float32(float(x)))
    if 'e' in s:
        s = np.format_float_positional(np.float32(float(x)), trim='-')
    if '.' not in s:
        s += '.0'
    return s


def vec(x):
    return ' '.join(f32(v) for v in x.split())


def camel(sym, strip):
    s = sym[len(strip):] if sym.startswith(strip) else sym
    return ''.join(p.capitalize() for p in s.lower().split('_') if p)


# ---- dumps ------------------------------------------------------------------
consts = []  # (name, kind, value, ref)
for l in open(os.path.join(T, 'consts.tsv')).read().splitlines():
    c = l.split('\t')
    consts.append((c[0], c[1], c[2], c[3]))
enum = {}  # type -> [(sym, value, ref)]
cval = {}
for n, k, v, r in consts:
    cval[n] = v
    if k.startswith('enum:'):
        enum.setdefault(k[5:], []).append((n, int(v), r))
tables = {}
cur = None
for l in open(os.path.join(T, 'tables.tsv')).read().splitlines():
    if l.startswith('@'):
        cur = l[1:]
        tables[cur] = []
        hdr = None
        continue
    if hdr is None:
        hdr = l.split('\t')
        continue
    tables[cur].append(dict(zip(hdr, l.split('\t'))))

bg = open(BG11).read().splitlines()


def ref(sym, array):
    """bg_misc.c line of the table entry whose first field is `sym`."""
    start = next(i for i, l in enumerate(bg) if re.match(r'\w+\s+' + array + r'\[\s*\]', l))
    for i in range(start, len(bg)):
        if re.match(r'\s*' + sym + r'\s*,', bg[i]):
            return f'bg_misc.c:{i + 1}'
    return 'NULL'


def sym_of(t, v):
    return next(s for s, n, _ in enum[t] if n == v)


def stages(v):
    v = int(v)
    return '|'.join(s for s, b in (('S1', 1), ('S2', 2), ('S3', 4)) if v & b) or 'NULL'


SLOTS = [('Head', 1), ('Torso', 2), ('Arms', 4), ('Legs', 8), ('Backpack', 16), ('Weapon', 32), ('Sidearm', 64)]
ABIL = [(camel(n, 'SCA_'), int(v)) for n, k, v, r in consts if n.startswith('SCA_')]


def flags(v, dom):
    v = int(v)
    return '|'.join(s for s, b in dom if v & b) or 'NULL'


def team_wut(v):
    return {0: 'None', 1: 'Aliens', 2: 'Humans'}[int(v)]


wname = {int(r['num']): r['name'] for r in tables['weapons']}
cname = {int(r['num']): r['name'] for r in tables['classes']}
s = lambda x: 'NULL' if x in ('', 'NULL') else x

sheets = {}
# ---- classes11 ----------------------------------------------------------------
rows = []
for r in tables['classes']:
    n = int(r['num'])
    kids = [cname[int(r[f'child{i}'])] for i in range(3) if int(r[f'child{i}']) != 0]
    sym = sym_of('pClass_t', n)
    rows.append([r['name'], str(n), r['name'], 'None' if n == 0 else ('Humans' if n >= int(cval['PCL_HUMAN']) else 'Aliens'),
                 stages(r['stages']), r['health'], f32(r['fall_damage']), r['regen_rate'], flags(r['abilities'], ABIL),
                 wname.get(int(r['start_weapon']), 'NULL'), f32(r['build_dist']), r['fov'], f32(r['bob']), f32(r['bob_cycle']),
                 r['steptime'], f32(r['speed']), f32(r['acceleration']), f32(r['air_acceleration']), f32(r['friction']),
                 f32(r['stop_speed']), f32(r['jump_magnitude']), f32(r['knockback_scale']), ' '.join(kids) or 'NULL', r['cost'],
                 r['value'], vec(r['mins']), vec(r['maxs']), vec(r['crouch_maxs']), vec(r['dead_mins']), vec(r['dead_maxs']),
                 r['viewheight'], r['crouch_viewheight'], f32(r['z_offset']), r['human_name'], s(r['model']), f32(r['model_scale']),
                 s(r['skin']), f32(r['shadow_scale']), s(r['hud']), sym, ref(sym, 'bg_classList')])
sheets['classes11'] = ('player classes of Tremulous 1.1.0 (bg_misc.c bg_classList with data-1.1.0.pk3 overrides/classes)', 'Class11Id', 'classes,weapons11', [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0), ('legacy', 'string', '-', 0, 'classes.id', 0), ('team', 'enum:Team', '-', 0, '-', 0),
    ('stages', 'flags:Stage', '-', 1, '-', 0), ('health', 'i32', 'hp', 0, '-', 0), ('fall_damage', 'f32', 'x', 0, '-', 0), ('regen_rate', 'i32', 'hp/s', 0, '-', 0),
    ('abilities', 'flags:Ability11', '-', 1, '-', 0), ('start_weapon', 'string', '-', 1, 'weapons11.id', 0), ('build_dist', 'f32', 'qu', 0, '-', 0),
    ('fov', 'i32', 'deg', 0, '-', 0), ('bob', 'f32', '-', 0, '-', 0), ('bob_cycle', 'f32', '-', 0, '-', 0), ('steptime', 'i32', 'ms', 0, '-', 0),
    ('speed', 'f32', 'x', 0, '-', 0), ('acceleration', 'f32', '-', 0, '-', 0), ('air_acceleration', 'f32', '-', 0, '-', 0), ('friction', 'f32', '-', 0, '-', 0),
    ('stop_speed', 'f32', 'qu/s', 0, '-', 0), ('jump_magnitude', 'f32', 'qu/s', 0, '-', 0), ('knockback_scale', 'f32', 'x', 0, '-', 0),
    ('children', 'list:string', '-', 1, 'classes11.id', 0), ('cost', 'i32', 'frags', 0, '-', 0), ('value', 'i32', 'credits', 0, '-', 0),
    ('mins', 'vec3', 'qu', 0, '-', 0), ('maxs', 'vec3', 'qu', 0, '-', 0), ('crouch_maxs', 'vec3', 'qu', 0, '-', 0), ('dead_mins', 'vec3', 'qu', 0, '-', 0),
    ('dead_maxs', 'vec3', 'qu', 0, '-', 0), ('viewheight', 'i32', 'qu', 0, '-', 0), ('crouch_viewheight', 'i32', 'qu', 0, '-', 0), ('z_offset', 'f32', 'qu', 0, '-', 0),
    ('display_name', 'string', '-', 0, '-', 1), ('model', 'string', '-', 1, '-', 1), ('model_scale', 'f32', 'x', 0, '-', 1), ('skin', 'string', '-', 1, '-', 1),
    ('shadow_scale', 'f32', 'x', 0, '-', 1), ('hud', 'string', '-', 1, '-', 1), ('c_sym', 'string', '-', 0, '-', 1), ('ref_src', 'string', '-', 1, '-', 1)], rows)

# ---- weapons11 ----------------------------------------------------------------
rows = []
for r in tables['weapons']:
    n = int(r['num'])
    sym = sym_of('weapon_t', n)
    rows.append([r['name'], str(n), r['name'], team_wut(r['team']), r['price'], stages(r['stages']), flags(r['slots'], SLOTS), r['max_ammo'],
                 r['max_clips'], r['infinite_ammo'], r['uses_energy'], r['repeat_rate1'], r['repeat_rate2'], r['repeat_rate3'], r['reload_time'],
                 r['has_alt_mode'], r['has_third_mode'], r['can_zoom'], f32(r['zoom_fov']), r['purchasable'], r['build_delay'], r['human_name'],
                 sym, ref(sym, 'bg_weapons')])
sheets['weapons11'] = ('weapons of Tremulous 1.1.0 (bg_misc.c bg_weapons)', 'Weapon11Id', 'weapons', [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0), ('legacy', 'string', '-', 0, 'weapons.id', 0), ('team', 'enum:Team', '-', 0, '-', 0),
    ('price', 'i32', 'credits', 0, '-', 0), ('stages', 'flags:Stage', '-', 1, '-', 0), ('slots', 'flags:Slot', '-', 1, '-', 0), ('max_ammo', 'i32', 'rounds', 0, '-', 0),
    ('max_clips', 'i32', 'clips', 0, '-', 0), ('infinite_ammo', 'bool', '-', 0, '-', 0), ('uses_energy', 'bool', '-', 0, '-', 0), ('repeat_rate1', 'i32', 'ms', 0, '-', 0),
    ('repeat_rate2', 'i32', 'ms', 0, '-', 0), ('repeat_rate3', 'i32', 'ms', 0, '-', 0), ('reload_time', 'i32', 'ms', 0, '-', 0), ('has_alt_mode', 'bool', '-', 0, '-', 0),
    ('has_third_mode', 'bool', '-', 0, '-', 0), ('can_zoom', 'bool', '-', 0, '-', 0), ('zoom_fov', 'f32', 'deg', 0, '-', 0), ('purchasable', 'bool', '-', 0, '-', 0),
    ('build_delay', 'i32', 'ms', 0, '-', 0), ('display_name', 'string', '-', 0, '-', 1), ('c_sym', 'string', '-', 0, '-', 1), ('ref_src', 'string', '-', 1, '-', 1)], rows)

# ---- upgrades11 ---------------------------------------------------------------
rows = []
for r in tables['upgrades']:
    n = int(r['num'])
    sym = sym_of('upgrade_t', n)
    rows.append([r['name'], str(n), r['name'], team_wut(r['team']), r['price'], stages(r['stages']), flags(r['slots'], SLOTS), r['purchasable'],
                 r['usable'], r['human_name'], s(r['icon']), sym, ref(sym, 'bg_upgrades')])
sheets['upgrades11'] = ('upgrades of Tremulous 1.1.0 (bg_misc.c bg_upgrades)', 'Upgrade11Id', 'upgrades', [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0), ('legacy', 'string', '-', 0, 'upgrades.id', 0), ('team', 'enum:Team', '-', 0, '-', 0),
    ('price', 'i32', 'credits', 0, '-', 0), ('stages', 'flags:Stage', '-', 1, '-', 0), ('slots', 'flags:Slot', '-', 1, '-', 0), ('purchasable', 'bool', '-', 0, '-', 0),
    ('usable', 'bool', '-', 0, '-', 0), ('display_name', 'string', '-', 0, '-', 1), ('icon', 'string', '-', 1, '-', 1), ('c_sym', 'string', '-', 0, '-', 1),
    ('ref_src', 'string', '-', 1, '-', 1)], rows)

# ---- buildables11 -------------------------------------------------------------
TRAJ = ['Stationary', 'Interpolate', 'Linear', 'LinearStop', 'Sine', 'Gravity', 'Buoyancy']
BANIM = {n: camel(sym, 'BANIM_') for sym, n, _ in enum['buildableAnimNumber_t']}
MOD = {n: sym for sym, n, _ in enum['meansOfDeath_t']}
rows = []
for r in tables['buildables']:
    n = int(r['num'])
    sym = sym_of('buildable_t', n)
    bw = int(r['build_weapon'])
    weapons = ' '.join(wname[i] for i in sorted(wname) if bw & (1 << i))
    proj = wname.get(int(r['turret_proj']), 'NULL')
    rows.append([r['name'], str(n), r['name'], team_wut(r['team']), TRAJ[int(r['traj'])], f32(r['bounce']), r['build_points'], stages(r['stages']),
                 r['health'], r['regen_rate'], r['splash_damage'], r['splash_radius'], camel(MOD[int(r['means_of_death'])], 'MOD_'), weapons,
                 BANIM[int(r['idle_anim'])], r['nextthink'], r['build_time'], r['usable'], r['turret_range'], r['turret_fire_speed'], proj,
                 f32(r['min_normal']), r['invert_normal'], r['creep_test'], r['creep_size'], r['dcc_test'], r['reactor_test'], vec(r['mins']),
                 vec(r['maxs']), f32(r['z_offset']), r['human_name'], r['entity_name'], s(r['model0']), f32(r['model_scale']), sym,
                 ref(sym, 'bg_buildableList')])
sheets['buildables11'] = ('structures of Tremulous 1.1.0 (bg_misc.c bg_buildableList with data-1.1.0.pk3 overrides/buildables)', 'Buildable11Id', 'buildables,weapons11', [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0), ('legacy', 'string', '-', 0, 'buildables.id', 0), ('team', 'enum:Team', '-', 0, '-', 0),
    ('traj', 'enum:Traj', '-', 0, '-', 0), ('bounce', 'f32', '-', 0, '-', 0), ('build_points', 'i32', 'bp', 0, '-', 0), ('stages', 'flags:Stage', '-', 1, '-', 0),
    ('health', 'i32', 'hp', 0, '-', 0), ('regen_rate', 'i32', 'hp/s', 0, '-', 0), ('splash_damage', 'i32', 'hp', 0, '-', 0), ('splash_radius', 'i32', 'qu', 0, '-', 0),
    ('means_of_death', 'enum:MeansOfDeath11', '-', 0, '-', 0), ('build_weapons', 'list:string', '-', 0, 'weapons11.id', 0), ('idle_anim', 'enum:BuildableAnim', '-', 0, '-', 0),
    ('nextthink', 'i32', 'ms', 0, '-', 0), ('build_time', 'i32', 'ms', 0, '-', 0), ('usable', 'bool', '-', 0, '-', 0), ('turret_range', 'i32', 'qu', 0, '-', 0),
    ('turret_fire_speed', 'i32', 'ms', 0, '-', 0), ('turret_proj', 'string', '-', 1, 'weapons11.id', 0), ('min_normal', 'f32', '-', 0, '-', 0),
    ('invert_normal', 'bool', '-', 0, '-', 0), ('creep_test', 'bool', '-', 0, '-', 0), ('creep_size', 'i32', 'qu', 0, '-', 0), ('dcc_test', 'bool', '-', 0, '-', 0),
    ('reactor_test', 'bool', '-', 0, '-', 0), ('mins', 'vec3', 'qu', 0, '-', 0), ('maxs', 'vec3', 'qu', 0, '-', 0), ('z_offset', 'f32', 'qu', 0, '-', 0),
    ('display_name', 'string', '-', 0, '-', 1), ('entity_name', 'string', '-', 0, '-', 1), ('model', 'string', '-', 1, '-', 1), ('model_scale', 'f32', 'x', 0, '-', 1),
    ('c_sym', 'string', '-', 0, '-', 1), ('ref_src', 'string', '-', 1, '-', 1)], rows)

# ---- constants11 --------------------------------------------------------------
rows = []
for n, k, v, r in consts:
    if k.startswith('enum:'):
        continue
    kind = {'i32': 'I32', 'f32': 'F32', 'f64': 'F64', 'str': 'Str'}[k]
    if kind == 'F32':
        v = f32(v)
    elif kind == 'F64':
        v = repr(float(v)) if '.' in repr(float(v)) else repr(float(v)) + '.0'
    rows.append([n.lower(), kind, v, r])
const_cols = [('id', 'string', '-', 0, '-', 0), ('kind', 'enum:ConstKind', '-', 0, '-', 0), ('value', 'string', '-', 0, '-', 0), ('ref_src', 'string', '-', 1, '-', 0)]

for name, (desc, dense, req, cols, rows_) in sheets.items():
    with open(os.path.join(S, 'domain', f'{name}.tsv'), 'w') as fh:
        fh.write(f'# sheet: {name}\n# version: 1\n# generator: table\n# index: by_id\n# requires: {req}\n# dense: {dense}\n'
                 f'# target: crate::gen::{name}\n# owned_by: synthesizer\n# doctrine: D1,D2,D3,D5\n# notes: {desc}; re/harness/synth_v11.py (dec041)\n')
        fh.write('\t'.join(c[0] for c in cols) + '\n')
        for r in rows_:
            assert len(r) == len(cols), (name, r)
            fh.write('\t'.join(r) + '\n')
with open(os.path.join(S, 'domain', 'constants11.tsv'), 'w') as fh:
    fh.write('# sheet: constants11\n# version: 1\n# generator: consts\n# index: by_id\n# requires: -\n# target: crate::gen::constants11\n'
             '# owned_by: synthesizer\n# doctrine: D1,D2,D3,D5\n# notes: Tremulous 1.1.0 tremulous.h / bg_public.h / bg_local.h / q_shared.h defines; re/harness/synth_v11.py (dec041)\n')
    fh.write('id\tkind\tvalue\tref_src\n')
    for r in rows:
        fh.write('\t'.join(r) + '\n')

# ---- enums --------------------------------------------------------------------
NEW = {'Ability11': None, 'Event11': ('entity_event_t', 'EV_'), 'MeansOfDeath11': ('meansOfDeath_t', 'MOD_'),
       'EntityType11': ('entityType_t', 'ET_'), 'Stat11': ('statIndex_t', 'STAT_'), 'Pers11': ('persEnum_t', 'PERS_'),
       'Menu11': ('dynMenu_t', 'MN_'), 'PmType11': ('pmtype_t', 'PM_')}
p = os.path.join(S, '01-enums.tsv')
lines = [l for l in open(p).read().splitlines() if l.split('\t')[0] not in NEW]
for dom, src in NEW.items():
    if src is None:
        for n, k, v, r in consts:
            if n.startswith('SCA_'):
                lines.append(f'{dom}\tflags\t{camel(n, "SCA_")}\t{v}\t{r}')
        continue
    t, strip = src
    for sym, v, r in enum[t]:
        lines.append(f'{dom}\tenum\t{camel(sym, strip)}\t{v}\t{r}')
open(p, 'w').write('\n'.join(lines) + '\n')

# ---- schema + groups ------------------------------------------------------------
p = os.path.join(S, '01-schema.tsv')
mine = set(sheets) | {'constants11'}
lines = [l for l in open(p).read().splitlines() if l.split('\t')[0] not in mine]
for name, (desc, dense, req, cols, rows_) in list(sheets.items()) + [('constants11', (None, None, None, const_cols, None))]:
    for c, ty, unit, opt, refc, cold in cols:
        lines.append(f'{name}\t{c}\t{ty}\t{unit}\t{opt}\t{refc}\t-\t{cold}')
open(p, 'w').write('\n'.join(lines) + '\n')
p = os.path.join(S, '00-groups.tsv')
lines = [l for l in open(p).read().splitlines() if l.split('\t')[0] not in mine]
for name, note in [('classes11', 'Tremulous 1.1.0 player classes (protocol 69 client, dec041)'), ('weapons11', 'Tremulous 1.1.0 weapons (dec041)'),
                   ('upgrades11', 'Tremulous 1.1.0 upgrades (dec041)'), ('buildables11', 'Tremulous 1.1.0 structures (dec041)'),
                   ('constants11', 'Tremulous 1.1.0 game constants (dec041)')]:
    lines.append(f'{name}\tEntities\t{note}')
open(p, 'w').write('\n'.join(lines) + '\n')
print('ok', {k: len(v[4]) for k, v in sheets.items()}, len(rows))
