#!/usr/bin/env python3
"""Seed sheets/domain/damage_regions.tsv from the original locdamage.cfg and
*.armour files (G_ParseDmgScript defaults). One-shot; the sheet is the truth after."""
import os, re, sys
ROOT = sys.argv[1]
A = os.path.join(ROOT, 're/source/assets')
def f32(x):
    s = repr(float(x));  return s if '.' in s else s + '.0'
def parse(path):
    txt = open(path).read()
    out = []
    for blk in re.findall(r'\{(.*?)\}', txt, re.S):
        r = dict(name='', minHeight=0.0, maxHeight=1.0, minAngle=0, maxAngle=360, modifier=1.0, crouch=0)
        for line in blk.strip().splitlines():
            t = line.split(None, 1)
            if not t: continue
            k = t[0]
            if k == 'crouch': r['crouch'] = 1
            elif k == 'name': r['name'] = t[1].strip().strip('"')
            elif k in ('minAngle', 'maxAngle'): r[k] = int(float(t[1]))
            else: r[k] = float(t[1])
        out.append(r)
    return out
rows = []
for cls in ('human_base', 'human_bsuit'):
    p = f'models/players/{cls}/locdamage.cfg'
    for r in parse(os.path.join(A, p)):
        rows.append([f"{cls}_{re.sub('[^a-z0-9]+', '_', r['name'].lower())}", 'Body', cls, 'NULL', r])
for up in ('larmour', 'helmet', 'bsuit'):
    for r in parse(os.path.join(A, f'armour/{up}.armour')):
        rows.append([re.sub('[^a-z0-9]+', '_', r['name'].lower()), 'Armour', 'NULL', up, r])
with open(os.path.join(ROOT, 'sheets/domain/damage_regions.tsv'), 'w') as f:
    f.write('# sheet: damage_regions\n# version: 1\n# generator: table\n# requires: classes,upgrades\n'
            '# target: crate::gen::damage_regions\n# owned_by: synthesizer\n'
            '# notes: locdamage.cfg + armour/*.armour (G_ParseDmgScript); order matters (file order)\n')
    f.write('id\tkind\tclass\tupgrade\tmin_height\tmax_height\tmin_angle\tmax_angle\tmodifier\tcrouch\n')
    for i, k, c, u, r in rows:
        f.write('\t'.join([i, k, c, u, f32(r['minHeight']), f32(r['maxHeight']), f32(r['minAngle']), f32(r['maxAngle']), f32(r['modifier']), str(r['crouch'])]) + '\n')
sch = [('id','string','-','0','-'),('kind','enum:RegionKind','-','0','-'),('class','string','-','1','classes.id'),('upgrade','string','-','1','upgrades.id'),
       ('min_height','f32','x','0','-'),('max_height','f32','x','0','-'),('min_angle','f32','deg','0','-'),('max_angle','f32','deg','0','-'),('modifier','f32','x','0','-'),('crouch','bool','-','0','-')]
with open(os.path.join(ROOT, 'sheets/01-schema.tsv'), 'a') as f:
    for c, t, u, o, r in sch: f.write(f'damage_regions\t{c}\t{t}\t{u}\t{o}\t{r}\t-\t0\n')
with open(os.path.join(ROOT, 'sheets/01-enums.tsv'), 'a') as f:
    f.write('RegionKind\tenum\tBody\t0\tg_combat.c:808\nRegionKind\tenum\tArmour\t1\tg_combat.c:808\n')
print(len(rows), 'regions')
