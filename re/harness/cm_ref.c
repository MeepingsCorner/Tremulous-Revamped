/* Collision oracle for real BSP files: the ORIGINAL cm_load.c / cm_trace.c /
 * cm_test.c / cm_patch.c / cm_polylib.c, compiled natively.
 *
 *   cm_ref map.bsp < queries.txt > results.txt
 *
 * Output starts with one line per patch surface (generated collide sizes and
 * bounds), then one line per query. Queries (all floats as 8-digit hex bits):
 *   T model sx sy sz ex ey ez mnx mny mnz mxx mxy mxz ox oy oz ax ay az mask
 *   C model px py pz ox oy oz ax ay az
 * model 0 uses CM_BoxTrace / CM_PointContents on the world; others use
 * CM_TransformedBoxTrace / CM_TransformedPointContents. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include "cm_local.h"
#include "cm_patch.h"

/* ---- engine shims ------------------------------------------------------ */
static char *hunk; static size_t hunk_used, hunk_size = (size_t)1 << 30;
void *Hunk_AllocDebug(int size, ha_pref p, char *l, char *f, int ln) {
  (void)p; (void)l; (void)f; (void)ln;
  if (!hunk) hunk = calloc(1, hunk_size);
  size = (size + 31) & ~31;
  if (hunk_used + size > hunk_size) { fprintf(stderr, "hunk full\n"); exit(4); }
  void *r = hunk + hunk_used; hunk_used += size; return r;
}
void *Z_MallocDebug(int size, char *l, char *f, int ln) { (void)l; (void)f; (void)ln; return calloc(1, size); }
void Z_Free(void *p) { free(p); }
static cvar_t cvars[8]; static int ncvars;
cvar_t *Cvar_Get(const char *name, const char *value, int flags) {
  cvar_t *c = &cvars[ncvars++]; (void)flags;
  c->name = (char *)name; c->string = (char *)value; c->value = atof(value); c->integer = atoi(value);
  return c;
}
unsigned Com_BlockChecksum(const void *b, int n) { (void)b; (void)n; return 0; }
void QDECL Com_Printf(const char *fmt, ...) { (void)fmt; }
void QDECL Com_DPrintf(const char *fmt, ...) { (void)fmt; }
void QDECL Com_Error(int level, const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); vfprintf(stderr, fmt, ap); va_end(ap);
  fprintf(stderr, "\n"); exit(5 + level);
}
long FS_ReadFile(const char *path, void **buffer) {
  FILE *f = fopen(path, "rb"); long n;
  if (!f) { *buffer = NULL; return -1; }
  fseek(f, 0, SEEK_END); n = ftell(f); fseek(f, 0, SEEK_SET);
  *buffer = malloc(n + 1); fread(*buffer, 1, n, f); fclose(f);
  return n;
}
void FS_FreeFile(void *b) { (void)b; /* the map stays referenced */ }

/* ---- io ------------------------------------------------------------------ */
static float hf(const char *s) { unsigned u = (unsigned)strtoul(s, NULL, 16); float f; memcpy(&f, &u, 4); return f; }
static unsigned fb(float f) { unsigned u; memcpy(&u, &f, 4); return u; }

int main(int argc, char **argv) {
  int checksum, i;
  char line[1024];
  if (argc < 2) { fprintf(stderr, "usage: cm_ref map.bsp < queries\n"); return 2; }
  CM_LoadMap(argv[1], qfalse, &checksum);
  for (i = 0; i < cm.numSurfaces; i++) {
    const patchCollide_t *pc;
    int j, k; unsigned h = 2166136261u;
    if (!cm.surfaces[i]) continue;
    pc = cm.surfaces[i]->pc;
    for (j = 0; j < pc->numPlanes; j++)
      for (k = 0; k < 4; k++) { h ^= fb(pc->planes[j].plane[k]); h *= 16777619u; }
    for (j = 0; j < pc->numFacets; j++) {
      const facet_t *f = &pc->facets[j];
      h ^= (unsigned)f->surfacePlane; h *= 16777619u;
      h ^= (unsigned)f->numBorders; h *= 16777619u;
      for (k = 0; k < f->numBorders; k++) {
        h ^= (unsigned)f->borderPlanes[k]; h *= 16777619u;
        h ^= (unsigned)f->borderInward[k]; h *= 16777619u;
      }
    }
    printf("P %d %d %d %08x %08x %08x %08x %08x %08x %08x\n", i, pc->numPlanes, pc->numFacets,
      fb(pc->bounds[0][0]), fb(pc->bounds[0][1]), fb(pc->bounds[0][2]),
      fb(pc->bounds[1][0]), fb(pc->bounds[1][1]), fb(pc->bounds[1][2]), h);
  }
  while (fgets(line, sizeof line, stdin)) {
    char *tok[32]; int n = 0; char *p = strtok(line, " \t\n");
    while (p && n < 32) { tok[n++] = p; p = strtok(NULL, " \t\n"); }
    if (n == 0) continue;
    if (tok[0][0] == 'T' && n == 21) {
      vec3_t s, e, mn, mx, o, a; trace_t tr; int model = atoi(tok[1]);
      for (i = 0; i < 3; i++) {
        s[i] = hf(tok[2 + i]); e[i] = hf(tok[5 + i]); mn[i] = hf(tok[8 + i]);
        mx[i] = hf(tok[11 + i]); o[i] = hf(tok[14 + i]); a[i] = hf(tok[17 + i]);
      }
      if (model == 0) CM_BoxTrace(&tr, s, e, mn, mx, 0, (int)strtol(tok[20], NULL, 16), TT_AABB);
      else CM_TransformedBoxTrace(&tr, s, e, mn, mx, model, (int)strtol(tok[20], NULL, 16), o, a, TT_AABB);
      printf("%08x %08x %08x %08x %08x %08x %08x %08x %d %d %x %x\n", fb(tr.fraction),
        fb(tr.endpos[0]), fb(tr.endpos[1]), fb(tr.endpos[2]),
        fb(tr.plane.normal[0]), fb(tr.plane.normal[1]), fb(tr.plane.normal[2]), fb(tr.plane.dist),
        tr.allsolid, tr.startsolid, tr.contents, tr.surfaceFlags);
    } else if (tok[0][0] == 'C' && n == 11) {
      vec3_t pt, o, a; int model = atoi(tok[1]), c;
      for (i = 0; i < 3; i++) { pt[i] = hf(tok[2 + i]); o[i] = hf(tok[5 + i]); a[i] = hf(tok[8 + i]); }
      c = model == 0 ? CM_PointContents(pt, 0) : CM_TransformedPointContents(pt, model, o, a);
      printf("%x\n", c);
    } else {
      fprintf(stderr, "bad query: %s\n", tok[0]); return 3;
    }
  }
  return 0;
}
