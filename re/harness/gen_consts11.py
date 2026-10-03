#!/usr/bin/env python3
"""Generate consts11.c: every object-like #define of Tremulous 1.1.0's
tremulous.h / bg_public.h (and the q_shared.h limits, buttons and reserved
configstrings) plus every member of bg_public.h's enums, evaluated by the C
compiler against the 1.1.0 headers (re/source11), so the 1.1 numbering the
port's v11 adapter relies on comes from the source (dec041)."""
import re, sys

defines, enums = [], []
WANT_QS = ('MAX_GENTITIES', 'MAX_CLIENTS', 'BUTTON_', 'MAX_STATS', 'MAX_PERSISTANT', 'MAX_POWERUPS',
           'MAX_WEAPONS', 'MAX_PS_EVENTS', 'ENTITYNUM_', 'CS_SERVERINFO', 'CS_SYSTEMINFO', 'MAX_CONFIGSTRINGS',
           'MAX_MODELS', 'MAX_SOUNDS', 'MAX_GAME_', 'MAX_LOCATIONS', 'SOLID_BMODEL', 'MOVE_RUN')
for path in sys.argv[1:]:
    base = path.split('/')[-1]
    text = open(path).read()
    lines = text.splitlines()
    for ln, line in enumerate(lines, 1):
        m = re.match(r'\s*#define\s+([A-Z_][A-Z0-9_]*)\s+(.+?)\s*(//.*)?$', line)
        if not m:
            continue
        name = m.group(1)
        if name.startswith('_') or name.endswith('_H') or '(' in name:
            continue
        if base == 'q_shared.h' and not name.startswith(WANT_QS):
            continue
        if base == 'bg_public.h' and name.startswith(('GAME_VERSION', 'MAX_ARENAS', 'MAX_BOTS', 'ARENAS_')):
            continue
        defines.append((name, f'{base}:{ln}'))
    if base != 'bg_public.h':
        continue
    # typedef enum { A, B = 3, ... } name_t;
    for m in re.finditer(r'typedef\s+enum\s*\{(.*?)\}\s*(\w+)\s*;', text, re.S):
        body = re.sub(r'//[^\n]*', '', m.group(1))
        body = re.sub(r'/\*.*?\*/', '', body, flags=re.S)
        start = text[:m.start()].count('\n') + 1
        for item in body.split(','):
            item = item.strip()
            if not item:
                continue
            name = item.split('=')[0].strip()
            blk = text[m.start():m.end()]
            ln = start + blk[:re.search(r'\b' + name + r'\b', blk).start()].count('\n')
            enums.append((name, m.group(2), f'{base}:{ln}'))

print('#include <stdio.h>\n#include "q_shared.h"\n#include "bg_public.h"\n#include "bg_local.h"')
print('#define EMIT(n, line) _Generic((n), float: pf, double: pd, char*: ps, const char*: ps, default: pi)(#n, line, (n))')
print('static void pf(const char*n,const char*l,float v){printf("%s\\tf32\\t%.9g\\t%s\\n",n,v,l);}')
print('static void pd(const char*n,const char*l,double v){printf("%s\\tf64\\t%.17g\\t%s\\n",n,v,l);}')
print('static void ps(const char*n,const char*l,const char*v){printf("%s\\tstr\\t%s\\t%s\\n",n,v,l);}')
print('static void pi(const char*n,const char*l,long v){printf("%s\\ti32\\t%ld\\t%s\\n",n,v,l);}')
print('static void pe(const char*n,const char*t,const char*l,long v){printf("%s\\tenum:%s\\t%ld\\t%s\\n",n,t,v,l);}')
print('int main(void){')
for n, ln in defines:
    print(f'  EMIT({n}, "{ln}");')
for n, t, ln in enums:
    print(f'  pe("{n}", "{t}", "{ln}", (long){n});')
print('  return 0;\n}')
