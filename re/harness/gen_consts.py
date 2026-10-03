#!/usr/bin/env python3
"""Generate consts.c: evaluates every object-like #define in tremulous.h with the
real C preprocessor/compiler, so constant values come from the source, not from
a re-implementation (MDD §7.2, L5 divergence oracle)."""
import re, sys

names = []
WANT_QS = ('ENTITYNUM_', 'MAX_GENTITIES', 'MAX_CLIENTS', 'BUTTON_', 'MAX_STATS', 'MAX_PERSISTANT', 'MAX_PS_EVENTS')
for path in sys.argv[1:]:
  base = path.split('/')[-1]
  src = open(path).read().splitlines()
  for ln, line in enumerate(src, 1):
    m = re.match(r'\s*#define\s+([A-Z_][A-Z0-9_]*)\s+(.+?)\s*(//.*)?$', line)
    if not m:
        continue
    name, body = m.group(1), m.group(2)
    if name.startswith('_') or name.endswith('_H') or '(' in name:
        continue
    if base == 'q_shared.h' and not name.startswith(WANT_QS):
        continue
    if base == 'bg_public.h' and name.startswith(('SCA_', 'SLOT_', 'GAME_VERSION', 'MAX_ARENAS', 'MAX_BOTS', 'ARENAS_', 'MAX_VOICE', 'VOICE_')):
        continue
    names.append((name, f'{base}:{ln}'))

print('#include <stdio.h>\n#include "q_shared.h"\n#include "bg_public.h"\n#include "bg_local.h"\n#include "tremulous.h"')
print('#define EMIT(n, line) _Generic((n), float: pf, double: pd, char*: ps, const char*: ps, default: pi)(#n, line, (n))')
print('static void pf(const char*n,const char*l,float v){printf("%s\\tf32\\t%.9g\\t%s\\n",n,v,l);}')
print('static void pd(const char*n,const char*l,double v){printf("%s\\tf64\\t%.17g\\t%s\\n",n,v,l);}')
print('static void ps(const char*n,const char*l,const char*v){printf("%s\\tstr\\t%s\\t%s\\n",n,v,l);}')
print('static void pi(const char*n,const char*l,long v){printf("%s\\ti32\\t%ld\\t%s\\n",n,v,l);}')
print('int main(void){')
for n, ln in names:
    print(f'  EMIT({n}, "{ln}");')
print('  return 0;\n}')
