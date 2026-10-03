#!/usr/bin/env python3
"""Mode A synthesizer (MDD §7, §8.6 analogue for source ports).

Reads the raw dumps produced by the ORIGINAL C code (extract, consts) plus the
original headers, and writes canonical TSV sheets (MDD §5.2) and the L5 evidence
relation. Run once to seed the sheet book; afterwards the sheets are the truth
(D1) and this script is only re-run to refresh re/evidence/ for L5 divergence.

usage: synth_sheets.py <repo_root> [--seed]
  --seed   also (over)write sheets/domain/*.tsv and their schema rows
"""
import os, re, sys
import numpy as np

ROOT = sys.argv[1]
SEED = '--seed' in sys.argv
SRC = os.path.join(ROOT, 're/source/src')
HERE = os.path.dirname(os.path.abspath(__file__))


def f32(x):
    s = str(np.float32(float(x)))
    if s in ('nan', 'inf', '-inf'):
        raise SystemExit('non-finite float')
    if 'e' in s:
        s = np.format_float_positional(np.float32(float(x)), trim='-')
    if '.' not in s:
        s += '.0'
    return s


def camel(sym, strip):
    s = sym[len(strip):] if sym.startswith(strip) else sym
    return ''.join(p.capitalize() for p in s.lower().split('_') if p)


def parse_enum(path, first):
    """Return [(symbol, value, line)] for the C enum containing `first`."""
    lines = open(path).read().splitlines()
    start = next(i for i, l in enumerate(lines) if re.match(r'\s*' + first + r'\s*[,=]?', l) and first in l)
    out, v = [], 0
    for i in range(start, len(lines)):
        l = lines[i].split('//')[0].strip()
        if l.startswith('}'):
            break
        m = re.match(r'([A-Z_0-9]+)\s*(=\s*([^,]+))?,?$', l)
        if not m:
            continue
        if m.group(3):
            v = int(eval(m.group(3)))
        out.append((m.group(1), v, i + 1))
        v += 1
    return out


def defines(path, prefix):
    out = []
    for i, l in enumerate(open(path).read().splitlines(), 1):
        m = re.match(r'#define\s+(' + prefix + r'[A-Z0-9_]+)\s+(0x[0-9a-fA-F]+|\d+)', l)
        if m:
            out.append((m.group(1), int(m.group(2), 0), i))
    return out


def raw_tables():
    t, cur, hdr = {}, None, None
    for line in open(os.path.join(HERE, 'raw_tables.tsv')).read().splitlines():
        if line.startswith('@'):
            cur, hdr = line[1:], None
            t[cur] = []
            continue
        cells = line.split('\t')
        if hdr is None:
            hdr = cells
            continue
        t[cur].append(dict(zip(hdr, cells)))
    return t


BGP = os.path.join(SRC, 'game/bg_public.h')
QSH = os.path.join(SRC, 'qcommon/q_shared.h')
BGM = os.path.join(SRC, 'game/bg_misc.c')
bgm_lines = open(BGM).read().splitlines()

E_CLASS = parse_enum(BGP, 'PCL_NONE')
E_WEAP = parse_enum(BGP, 'WP_NONE')
E_UPG = parse_enum(BGP, 'UP_NONE')
E_BUILD = parse_enum(BGP, 'BA_NONE')
E_TEAM = parse_enum(BGP, 'TEAM_NONE')
E_MOD = parse_enum(BGP, 'MOD_UNKNOWN')
E_BANIM = parse_enum(BGP, 'BANIM_NONE')
E_TR = parse_enum(QSH, 'TR_STATIONARY')
F_SCA = defines(BGP, 'SCA_')
F_SLOT = [d for d in defines(BGP, 'SLOT_') if d[1]]

ENUMS = [  # (rust enum name, kind, entries, strip-prefix, source file)
    ('Team', 'enum', [e for e in E_TEAM if not e[0].startswith('NUM_')], 'TEAM_', 'game/bg_public.h'),
    ('Traj', 'enum', E_TR, 'TR_', 'qcommon/q_shared.h'),
    ('MeansOfDeath', 'enum', E_MOD, 'MOD_', 'game/bg_public.h'),
    ('BuildableAnim', 'enum', [e for e in E_BANIM if not e[0].startswith('MAX_')], 'BANIM_', 'game/bg_public.h'),
    ('PmType', 'enum', parse_enum(BGP, 'PM_NORMAL'), 'PM_', 'game/bg_public.h'),
    ('WeaponState', 'enum', parse_enum(BGP, 'WEAPON_READY'), 'WEAPON_', 'game/bg_public.h'),
    ('Stat', 'enum', parse_enum(BGP, 'STAT_HEALTH'), 'STAT_', 'game/bg_public.h'),
    ('Pers', 'enum', parse_enum(BGP, 'PERS_SCORE'), 'PERS_', 'game/bg_public.h'),
    ('Event', 'enum', parse_enum(BGP, 'EV_NONE'), 'EV_', 'game/bg_public.h'),
    ('PlayerAnim', 'enum', parse_enum(BGP, 'BOTH_DEATH1'), '', 'game/bg_public.h'),
    ('NonSegAnim', 'enum', parse_enum(BGP, 'NSPA_STAND'), 'NSPA_', 'game/bg_public.h'),
    ('WeaponAnim', 'enum', parse_enum(BGP, 'WANIM_NONE'), 'WANIM_', 'game/bg_public.h'),
    ('WeaponMode', 'enum', parse_enum(BGP, 'WPM_NONE'), 'WPM_', 'game/bg_public.h'),
    ('SpectatorState', 'enum', parse_enum(BGP, 'SPECTATOR_NOT'), 'SPECTATOR_', 'game/bg_public.h'),
    ('Stage', 'flags', [('S1', 1, 0), ('S2', 2, 0), ('S3', 4, 0)], '', 'game/bg_public.h'),
    ('Ability', 'flags', F_SCA, 'SCA_', 'game/bg_public.h'),
    ('Slot', 'flags', F_SLOT, 'SLOT_', 'game/bg_public.h'),
]
ENUM_BY = {e[0]: e for e in ENUMS}


def enum_name(enum, value):
    _, kind, entries, strip, _ = ENUM_BY[enum]
    for sym, v, _ in entries:
        if v == value:
            return camel(sym, strip)
    raise SystemExit(f'{enum}: no variant {value}')


def flags_name(enum, value):
    _, kind, entries, strip, _ = ENUM_BY[enum]
    if value == 0:
        return 'NULL'
    names, rest = [], value
    for sym, v, _ in entries:
        if value & v:
            names.append(camel(sym, strip))
            rest &= ~v
    if rest:
        raise SystemExit(f'{enum}: unknown bits {rest:#x}')
    return '|'.join(names)


def table_line(sym):
    """Line in bg_misc.c where the table row for enum symbol `sym` starts."""
    for i, l in enumerate(bgm_lines):
        if re.match(r'\s*' + sym + r',\s*(//.*)?$', l) and '{' in bgm_lines[i - 1]:
            return f'bg_misc.c:{i + 1}'
    return 'NULL'


def by_num(entries):
    return {v: s for s, v, _ in entries}


T = raw_tables()
cls_sym, wp_sym, up_sym, ba_sym = by_num(E_CLASS), by_num(E_WEAP), by_num(E_UPG), by_num(E_BUILD)
wp_id = {int(r['num']): r['name'] for r in T['weapons']}
cls_id = {int(r['num']): r['name'] for r in T['classes']}


def team_of_class(num):
    sym = cls_sym[num]
    return 'Aliens' if 'ALIEN' in sym else 'Humans' if 'HUMAN' in sym else 'None'


def weapon_ref(n):
    return wp_id[n] if n else 'NULL'


def vec(s):
    return ' '.join(f32(x) for x in s.split())


def s_or_null(s):
    return s if s != '' else 'NULL'


# ---------------------------------------------------------------- sheets
# Each sheet: (name, manifest, columns[(name,type,unit,optional,ref,cold)], rows)
SHEETS = []

cols_classes = [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0),
    ('team', 'enum:Team', '-', 0, '-', 0), ('stages', 'flags:Stage', '-', 0, '-', 0),
    ('health', 'i32', 'hp', 0, '-', 0), ('fall_damage', 'f32', 'x', 0, '-', 0),
    ('regen_rate', 'f32', 'hp/s', 0, '-', 0), ('abilities', 'flags:Ability', '-', 1, '-', 0),
    ('start_weapon', 'string', '-', 1, 'weapons.id', 0), ('build_dist', 'f32', 'qu', 0, '-', 0),
    ('fov', 'i32', 'deg', 0, '-', 0), ('bob', 'f32', '-', 0, '-', 0), ('bob_cycle', 'f32', '-', 0, '-', 0),
    ('steptime', 'i32', 'ms', 0, '-', 0), ('speed', 'f32', 'x', 0, '-', 0),
    ('acceleration', 'f32', '-', 0, '-', 0), ('air_acceleration', 'f32', '-', 0, '-', 0),
    ('friction', 'f32', '-', 0, '-', 0), ('stop_speed', 'f32', 'qu/s', 0, '-', 0),
    ('jump_magnitude', 'f32', 'qu/s', 0, '-', 0), ('knockback_scale', 'f32', 'x', 0, '-', 0),
    ('evolves_to', 'list:string', '-', 1, 'classes.id', 0), ('cost', 'i32', 'evos', 0, '-', 0),
    ('value', 'i32', 'credits', 0, '-', 0),
    ('mins', 'vec3', 'qu', 0, '-', 0), ('maxs', 'vec3', 'qu', 0, '-', 0),
    ('crouch_maxs', 'vec3', 'qu', 0, '-', 0), ('dead_mins', 'vec3', 'qu', 0, '-', 0),
    ('dead_maxs', 'vec3', 'qu', 0, '-', 0), ('viewheight', 'i32', 'qu', 0, '-', 0),
    ('crouch_viewheight', 'i32', 'qu', 0, '-', 0), ('z_offset', 'f32', 'qu', 0, '-', 0),
    ('shoulder_offsets', 'vec3', 'qu', 0, '-', 1),
    ('display_name', 'string', '-', 0, '-', 1), ('model', 'string', '-', 1, '-', 1),
    ('model_scale', 'f32', 'x', 0, '-', 1), ('skin', 'string', '-', 1, '-', 1),
    ('shadow_scale', 'f32', 'x', 0, '-', 1), ('hud', 'string', '-', 1, '-', 1),
    ('c_sym', 'string', '-', 0, '-', 1), ('ref_src', 'string', '-', 1, '-', 1),
]
rows = []
for r in T['classes']:
    n = int(r['num'])
    kids = [cls_id[int(r[f'child{k}'])] for k in range(3) if int(r[f'child{k}'])]
    rows.append([
        r['name'], str(n), team_of_class(n), flags_name('Stage', int(r['stages'])), r['health'],
        f32(r['fall_damage']), f32(r['regen_rate']), flags_name('Ability', int(r['abilities'])),
        weapon_ref(int(r['start_weapon'])), f32(r['build_dist']), r['fov'], f32(r['bob']),
        f32(r['bob_cycle']), r['steptime'], f32(r['speed']), f32(r['acceleration']),
        f32(r['air_acceleration']), f32(r['friction']), f32(r['stop_speed']), f32(r['jump_magnitude']),
        f32(r['knockback_scale']), ' '.join(kids) if kids else 'NULL', r['cost'], r['value'],
        vec(r['mins']), vec(r['maxs']), vec(r['crouch_maxs']), vec(r['dead_mins']), vec(r['dead_maxs']),
        r['viewheight'], r['crouch_viewheight'], f32(r['z_offset']), vec(r['shoulder_offsets']),
        r['human_name'], s_or_null(r['model']), f32(r['model_scale']), s_or_null(r['skin']),
        f32(r['shadow_scale']), s_or_null(r['hud']), cls_sym[n], table_line(cls_sym[n])])
SHEETS.append(('classes', {'generator': 'table', 'index': 'by_id', 'requires': 'weapons',
                           'dense': 'ClassId'}, cols_classes, rows))

cols_weapons = [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0), ('team', 'enum:Team', '-', 0, '-', 0),
    ('price', 'i32', 'credits', 0, '-', 0), ('stages', 'flags:Stage', '-', 0, '-', 0),
    ('slots', 'flags:Slot', '-', 1, '-', 0), ('max_ammo', 'i32', 'rounds', 0, '-', 0),
    ('max_clips', 'i32', 'clips', 0, '-', 0), ('infinite_ammo', 'bool', '-', 0, '-', 0),
    ('uses_energy', 'bool', '-', 0, '-', 0), ('repeat_rate1', 'i32', 'ms', 0, '-', 0),
    ('repeat_rate2', 'i32', 'ms', 0, '-', 0), ('repeat_rate3', 'i32', 'ms', 0, '-', 0),
    ('reload_time', 'i32', 'ms', 0, '-', 0), ('knockback_scale', 'f32', 'x', 0, '-', 0),
    ('has_alt_mode', 'bool', '-', 0, '-', 0), ('has_third_mode', 'bool', '-', 0, '-', 0),
    ('can_zoom', 'bool', '-', 0, '-', 0), ('zoom_fov', 'f32', 'deg', 0, '-', 0),
    ('purchasable', 'bool', '-', 0, '-', 0), ('long_ranged', 'bool', '-', 0, '-', 0),
    ('display_name', 'string', '-', 0, '-', 1), ('c_sym', 'string', '-', 0, '-', 1),
    ('ref_src', 'string', '-', 1, '-', 1),
]
rows = []
for r in T['weapons']:
    n = int(r['num'])
    rows.append([r['name'], str(n), enum_name('Team', int(r['team'])), r['price'],
                 flags_name('Stage', int(r['stages'])), flags_name('Slot', int(r['slots'])), r['max_ammo'],
                 r['max_clips'], r['infinite_ammo'], r['uses_energy'], r['repeat_rate1'], r['repeat_rate2'],
                 r['repeat_rate3'], r['reload_time'], f32(r['knockback_scale']), r['has_alt_mode'],
                 r['has_third_mode'], r['can_zoom'], f32(r['zoom_fov']), r['purchasable'], r['long_ranged'],
                 r['human_name'], wp_sym[n], table_line(wp_sym[n])])
SHEETS.append(('weapons', {'generator': 'table', 'index': 'by_id', 'requires': '-', 'dense': 'WeaponId'},
               cols_weapons, rows))

cols_upgrades = [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0), ('team', 'enum:Team', '-', 0, '-', 0),
    ('price', 'i32', 'credits', 0, '-', 0), ('stages', 'flags:Stage', '-', 0, '-', 0),
    ('slots', 'flags:Slot', '-', 1, '-', 0), ('purchasable', 'bool', '-', 0, '-', 0),
    ('usable', 'bool', '-', 0, '-', 0), ('display_name', 'string', '-', 0, '-', 1),
    ('icon', 'string', '-', 1, '-', 1), ('c_sym', 'string', '-', 0, '-', 1), ('ref_src', 'string', '-', 1, '-', 1),
]
rows = []
for r in T['upgrades']:
    n = int(r['num'])
    rows.append([r['name'], str(n), enum_name('Team', int(r['team'])), r['price'],
                 flags_name('Stage', int(r['stages'])), flags_name('Slot', int(r['slots'])), r['purchasable'],
                 r['usable'], r['human_name'], s_or_null(r['icon']), up_sym[n], table_line(up_sym[n])])
SHEETS.append(('upgrades', {'generator': 'table', 'index': 'by_id', 'requires': '-', 'dense': 'UpgradeId'},
               cols_upgrades, rows))

cols_build = [
    ('id', 'string', '-', 0, '-', 0), ('num', 'u8', '-', 0, '-', 0), ('team', 'enum:Team', '-', 0, '-', 0),
    ('traj', 'enum:Traj', '-', 0, '-', 0), ('bounce', 'f32', '-', 0, '-', 0),
    ('build_points', 'i32', 'bp', 0, '-', 0), ('stages', 'flags:Stage', '-', 0, '-', 0),
    ('health', 'i32', 'hp', 0, '-', 0), ('regen_rate', 'i32', 'hp/s', 0, '-', 0),
    ('splash_damage', 'i32', 'hp', 0, '-', 0), ('splash_radius', 'i32', 'qu', 0, '-', 0),
    ('means_of_death', 'enum:MeansOfDeath', '-', 0, '-', 0),
    ('build_weapons', 'list:string', '-', 0, 'weapons.id', 0),
    ('idle_anim', 'enum:BuildableAnim', '-', 0, '-', 0), ('nextthink', 'i32', 'ms', 0, '-', 0),
    ('build_time', 'i32', 'ms', 0, '-', 0), ('usable', 'bool', '-', 0, '-', 0),
    ('turret_range', 'i32', 'qu', 0, '-', 0), ('turret_fire_speed', 'i32', 'ms', 0, '-', 0),
    ('turret_proj', 'string', '-', 1, 'weapons.id', 0), ('min_normal', 'f32', '-', 0, '-', 0),
    ('invert_normal', 'bool', '-', 0, '-', 0), ('creep_test', 'bool', '-', 0, '-', 0),
    ('creep_size', 'i32', 'qu', 0, '-', 0), ('dcc_test', 'bool', '-', 0, '-', 0),
    ('transparent_test', 'bool', '-', 0, '-', 0), ('unique_test', 'bool', '-', 0, '-', 0),
    ('value', 'i32', 'credits', 0, '-', 0), ('mins', 'vec3', 'qu', 0, '-', 0), ('maxs', 'vec3', 'qu', 0, '-', 0),
    ('z_offset', 'f32', 'qu', 0, '-', 0), ('display_name', 'string', '-', 0, '-', 1),
    ('entity_name', 'string', '-', 0, '-', 1), ('model', 'string', '-', 1, '-', 1),
    ('model_scale', 'f32', 'x', 0, '-', 1), ('c_sym', 'string', '-', 0, '-', 1),
    ('ref_src', 'string', '-', 1, '-', 1),
]
rows = []
for r in T['buildables']:
    n = int(r['num'])
    bw = [wp_id[i] for i in sorted(wp_id) if int(r['build_weapon']) & (1 << i)]
    rows.append([r['name'], str(n), enum_name('Team', int(r['team'])), enum_name('Traj', int(r['traj'])),
                 f32(r['bounce']), r['build_points'], flags_name('Stage', int(r['stages'])), r['health'],
                 r['regen_rate'], r['splash_damage'], r['splash_radius'],
                 enum_name('MeansOfDeath', int(r['means_of_death'])), ' '.join(bw),
                 enum_name('BuildableAnim', int(r['idle_anim'])), r['nextthink'], r['build_time'], r['usable'],
                 r['turret_range'], r['turret_fire_speed'], weapon_ref(int(r['turret_proj'])),
                 f32(r['min_normal']), r['invert_normal'], r['creep_test'], r['creep_size'], r['dcc_test'],
                 r['transparent_test'], r['unique_test'], r['value'], vec(r['mins']), vec(r['maxs']),
                 f32(r['z_offset']), r['human_name'], r['entity_name'], s_or_null(r['model0']),
                 f32(r['model_scale']), ba_sym[n], table_line(ba_sym[n])])
SHEETS.append(('buildables', {'generator': 'table', 'index': 'by_id', 'requires': 'weapons',
                              'dense': 'BuildableId'}, cols_build, rows))

cols_consts = [('id', 'string', '-', 0, '-', 0), ('kind', 'enum:ConstKind', '-', 0, '-', 0),
               ('value', 'string', '-', 0, '-', 0), ('ref_src', 'string', '-', 1, '-', 1)]
rows = []
for line in open(os.path.join(HERE, 'raw_consts.tsv')).read().splitlines():
    name, ty, val, ln = line.split('\t')
    if ty == 'f64':
        v = repr(float(val))
        val = v if '.' in v else v + '.0'
    rows.append([name.lower(), {'f32': 'F32', 'f64': 'F64', 'str': 'Str'}.get(ty, 'I32'), f32(val) if ty == 'f32' else val, ln])
SHEETS.append(('constants', {'generator': 'consts', 'index': 'by_id', 'requires': '-'}, cols_consts, rows))


# ---------------------------------------------------------------- writers
def write_tsv(path, manifest, header, rows):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w', newline='\n') as f:
        for k, v in manifest.items():
            f.write(f'# {k}: {v}\n')
        f.write('\t'.join(header) + '\n')
        for r in rows:
            assert len(r) == len(header), (path, r)
            f.write('\t'.join(str(c) for c in r) + '\n')


def evidence():
    """L5 relation: (sheet, id, column, value) straight from the original code."""
    out = []
    for name, _, cols, rows in SHEETS:
        for r in rows:
            for (c, *_), v in zip(cols, r):
                if c in ('id', 'ref_src', 'c_sym'):
                    continue
                out.append([name, r[0], c, v])
    write_tsv(os.path.join(ROOT, 're/evidence/l5_reference.tsv'),
              {'sheet': 'l5_reference', 'generator': 'none', 'source': 'harness extract+consts @ c862a534'},
              ['sheet', 'row', 'column', 'value'], out)


evidence()
if SEED:
    for name, man, cols, rows in SHEETS:
        m = {'sheet': name, 'version': 1}
        m.update(man)
        m.update({'target': f'crate::gen::{name}', 'owned_by': 'synthesizer', 'doctrine': 'D1,D2,D3,D5'})
        write_tsv(os.path.join(ROOT, f'sheets/domain/{name}.tsv'), m, [c[0] for c in cols], rows)
    # enums sheet
    erows = [['ConstKind', 'enum', 'I32', '0', 'NULL'], ['ConstKind', 'enum', 'F32', '1', 'NULL'], ['ConstKind', 'enum', 'Str', '2', 'NULL'], ['ConstKind', 'enum', 'F64', '3', 'NULL']]
    for en, kind, entries, strip, src in ENUMS:
        for sym, v, ln in entries:
            erows.append([en, kind, camel(sym, strip), str(v), f'{os.path.basename(src)}:{ln}' if ln else 'NULL'])
    ep = os.path.join(ROOT, 'sheets/01-enums.tsv')
    mine = {'ConstKind'} | {e[0] for e in ENUMS}
    if os.path.exists(ep):
        body = [l for l in open(ep).read().splitlines() if l and not l.startswith('#')][1:]
        erows += [l.split('\t') for l in body if l.split('\t')[0] not in mine]
    write_tsv(os.path.join(ROOT, 'sheets/01-enums.tsv'),
              {'sheet': 'enums', 'version': 1, 'generator': 'enums', 'target': 'crate::gen::enums',
               'owned_by': 'architect'}, ['enum', 'kind', 'variant', 'value', 'ref_src'], erows)
    # schema rows for synthesized sheets (other sheets' rows preserved)
    sp = os.path.join(ROOT, 'sheets/01-schema.tsv')
    keep, man, hdr = [], {}, ['sheet', 'column', 'type', 'unit', 'optional', 'ref', 'default', 'cold']
    if os.path.exists(sp):
        lines = open(sp).read().splitlines()
        for l in lines:
            if l.startswith('#'):
                k, v = l[1:].split(':', 1)
                man[k.strip()] = v.strip()
        body = [l for l in lines if not l.startswith('#')]
        keep = [l.split('\t') for l in body[1:] if l and l.split('\t')[0] not in {s[0] for s in SHEETS}]
    man = man or {'sheet': 'schema', 'version': 1, 'generator': 'none', 'owned_by': 'architect'}
    srows = keep
    for name, _, cols, _ in SHEETS:
        for c, ty, unit, opt, ref, cold in cols:
            srows.append([name, c, ty, unit, str(opt), ref, '-', str(cold)])
    write_tsv(sp, man, hdr, srows)
print('ok', 'seeded' if SEED else 'evidence only')
