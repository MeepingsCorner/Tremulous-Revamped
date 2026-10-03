#!/usr/bin/env python3
"""One-shot authoring aid for the `arena` map: computes cut-plane normals and
line-of-sight waypoint links, then writes rows to sheets. After seeding, the
sheets are the truth (D1); edit them, not this script."""
import math, os, sys

ROOT = sys.argv[1]


def fl(x):
    s = ('%.6f' % x).rstrip('0')
    return s + '0' if s.endswith('.') else s


def v3(a):
    return ' '.join(fl(float(x)) for x in a)


B = []  # id, mins, maxs, cut, contents, surface, material


def box(i, mins, maxs, mat, cut=None, contents='Solid', surface='NULL'):
    B.append([i, 'arena', v3(mins), v3(maxs), cut or 'NULL', contents, surface, mat])


def ramp(i, x0, x1, y0, y1, z0, h, mat, axis='x', rising=+1):
    """Wedge rising along +axis (rising=+1) or -axis from z0 to z0+h."""
    if axis == 'x':
        L = x1 - x0
        nx = -h if rising > 0 else h
        n = (nx, 0.0, L)
        p = (x0 if rising > 0 else x1, 0.0, z0)
    else:
        L = y1 - y0
        ny = -h if rising > 0 else h
        n = (0.0, ny, L)
        p = (0.0, y0 if rising > 0 else y1, z0)
    m = math.sqrt(sum(c * c for c in n))
    n = tuple(c / m for c in n)
    d = sum(a * b for a, b in zip(n, p))
    box(i, (x0, y0, z0), (x1, y1, z0 + h), mat, cut=v3((*n, d)))


W, H, Z, T = 1536, 1024, 512, 32
box('floor', (-W, -H, -T), (W, H, 0), 'floor')
box('ceiling', (-W, -H, Z), (W, H, Z + T), 'ceiling')
box('wall_n', (-W, H, 0), (W, H + T, Z), 'wall')
box('wall_s', (-W, -H - T, 0), (W, -H, Z), 'wall')
box('wall_w', (-W - T, -H, 0), (-W, H, Z), 'wall')
box('wall_e', (W, -H, 0), (W + T, H, Z), 'wall')
# divider walls with doorways (gap |y| < 128, plus a high window)
for side, x in (('w', -640), ('e', 640)):
    box(f'div_{side}_n', (x - 16, 128, 0), (x + 16, H, Z), 'wall')
    box(f'div_{side}_s', (x - 16, -H, 0), (x + 16, -128, Z), 'wall')
    box(f'div_{side}_lintel', (x - 16, -128, 160), (x + 16, 128, Z), 'trim')
# central platform with ramps on east and west
box('plat', (-160, -160, 0), (160, 160, 64), 'metal')
ramp('plat_ramp_w', -352, -160, -96, 96, 0, 64, 'metal', 'x', +1)
ramp('plat_ramp_e', 160, 352, -96, 96, 0, 64, 'metal', 'x', -1)
# pillars
for i, (x, y) in enumerate(((-400, -560), (-400, 560), (400, -560), (400, 560))):
    box(f'pillar_{i}', (x - 40, y - 40, 0), (x + 40, y + 40, Z), 'trim')
# north balcony reached by a ramp, plus a ladder on the south wall
box('balcony', (-480, 768, 160), (480, H, 192), 'metal')
# a ramp along -y up to the balcony edge: from y=448 (z=0) to y=768 (z=160)
ramp('balcony_ramp', -96, 96, 448, 768, 0, 160, 'metal', 'y', +1)
box('ladder', (-64, -H, 0), (64, -H + 8, 320), 'ladder', surface='Ladder')
box('ledge_s', (-256, -H, 320), (256, -H + 160, 352), 'metal')
# low cover crates
for i, (x, y) in enumerate(((-900, 500), (-900, -500), (900, 500), (900, -500), (0, -560), (0, 380))):
    box(f'crate_{i}', (x - 48, y - 48, 0), (x + 48, y + 48, 72), 'crate')

E = []  # id, map, kind, team, buildable, origin, yaw
def ent(i, kind, team, b, o, yaw=0.0):
    E.append([i, 'arena', kind, team, b or 'NULL', v3(o), fl(float(yaw))])

ent('a_overmind', 'Buildable', 'Aliens', 'overmind', (-1300, 0, 64), 0)
ent('a_egg_n', 'Buildable', 'Aliens', 'eggpod', (-1150, 260, 64), 0)
ent('a_egg_s', 'Buildable', 'Aliens', 'eggpod', (-1150, -260, 64), 0)
ent('a_egg_w', 'Buildable', 'Aliens', 'eggpod', (-1400, 400, 64), 0)
ent('a_acid', 'Buildable', 'Aliens', 'acid_tube', (-800, 200, 64), 0)
ent('a_booster', 'Buildable', 'Aliens', 'booster', (-1400, -450, 64), 0)
ent('h_reactor', 'Buildable', 'Humans', 'reactor', (1300, 0, 64), 180)
ent('h_node_n', 'Buildable', 'Humans', 'telenode', (1150, 260, 64), 180)
ent('h_node_s', 'Buildable', 'Humans', 'telenode', (1150, -260, 64), 180)
ent('h_node_e', 'Buildable', 'Humans', 'telenode', (1400, 400, 64), 180)
ent('h_armoury', 'Buildable', 'Humans', 'arm', (1380, -420, 64), 180)
ent('h_medi', 'Buildable', 'Humans', 'medistat', (1050, -600, 64), 180)
ent('h_turret_n', 'Buildable', 'Humans', 'mgturret', (820, 220, 64), 180)
ent('h_turret_s', 'Buildable', 'Humans', 'mgturret', (820, -220, 64), 180)
ent('spec_start', 'PlayerStart', 'None', None, (0, 0, 400), 0)
ent('a_intermission', 'Intermission', 'Aliens', None, (-1100, 0, 300), 0)
ent('h_intermission', 'Intermission', 'Humans', None, (1100, 0, 300), 180)

# waypoints: grid points not inside brushes, linked by clear line of sight
solid = [(tuple(map(float, b[2].split())), tuple(map(float, b[3].split()))) for b in B]
def inside(p, pad=20):
    return any(all(mn[k] - pad < p[k] < mx[k] + pad for k in range(3)) for mn, mx in solid)
def seg_clear(a, b):
    for t in range(1, 40):
        p = tuple(a[k] + (b[k] - a[k]) * t / 40 for k in range(3))
        if inside(p, 18):
            return False
    return True
pts = []
for x in range(-1400, 1401, 200):
    for y in range(-900, 901, 200):
        p = (float(x), float(y), 24.0)
        if not inside(p):
            pts.append(p)
pts += [(0.0, 0.0, 88.0), (-256.0, 0.0, 56.0), (256.0, 0.0, 56.0)]
W_ = []
for i, p in enumerate(pts):
    links = []
    for j, q in enumerate(pts):
        if i != j and math.dist(p, q) <= 300 and abs(p[2] - q[2]) < 70 and seg_clear(p, q):
            links.append(f'wp{j}')
    W_.append([f'wp{i}', 'arena', v3(p), ' '.join(links) if links else 'NULL'])
W_ = [w for w in W_ if w[3] != 'NULL']
keep = {w[0] for w in W_}
for w in W_:
    w[3] = ' '.join(l for l in w[3].split() if l in keep)

def write(name, manifest, header, rows):
    p = os.path.join(ROOT, 'sheets/domain', name + '.tsv')
    with open(p, 'w') as fh:
        for k, v in manifest:
            fh.write(f'# {k}: {v}\n')
        fh.write('\t'.join(header) + '\n')
        for r in rows:
            fh.write('\t'.join(r) + '\n')

write('brushes', [('sheet', 'brushes'), ('version', 1), ('generator', 'table'), ('index', 'by_id'),
                  ('requires', 'maps,materials'), ('target', 'crate::gen::brushes'), ('owned_by', 'level-design')],
      ['id', 'map', 'mins', 'maxs', 'cut', 'contents', 'surface', 'material'], B)
write('map_entities', [('sheet', 'map_entities'), ('version', 1), ('generator', 'table'), ('requires', 'maps,buildables'),
                       ('target', 'crate::gen::map_entities'), ('owned_by', 'level-design')],
      ['id', 'map', 'kind', 'team', 'buildable', 'origin', 'yaw'], E)
write('waypoints', [('sheet', 'waypoints'), ('version', 1), ('generator', 'table'), ('dense', 'WaypointId'),
                    ('cyclic', 'links'), ('requires', 'maps'), ('target', 'crate::gen::waypoints'), ('owned_by', 'level-design')],
      ['id', 'map', 'origin', 'links'], W_)
print(len(B), 'brushes', len(E), 'entities', len(W_), 'waypoints')
