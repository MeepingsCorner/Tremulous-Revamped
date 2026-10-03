/* Builds the ORIGINAL clipMap_t (cm_trace.c / cm_test.c) from the sheet book's
 * brush rows, in exactly the layout kernel cm.rs uses: per brush the sides are
 * +x,-x,+y,-y,+z,-z[,cut]; one node whose children are both leaf 0. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "cm_local.h"

clipMap_t cm;
int c_pointcontents, c_traces, c_brush_traces, c_patch_traces;
static cvar_t cvar_zero;
cvar_t *cm_noAreas = &cvar_zero, *cm_noCurves = &cvar_zero, *cm_playerCurveClip = &cvar_zero;
static cmodel_t world_model;

cmodel_t *CM_ClipHandleToModel(clipHandle_t h) { (void)h; return &world_model; }
void CM_ModelBounds(clipHandle_t m, vec3_t mins, vec3_t maxs) { (void)m; VectorClear(mins); VectorClear(maxs); }
clipHandle_t CM_TempBoxModel(const vec3_t a, const vec3_t b, int c) { (void)a; (void)b; (void)c; return 0; }
void CM_TraceThroughPatchCollide(traceWork_t *tw, const struct patchCollide_s *pc) { (void)tw; (void)pc; }
qboolean CM_PositionTestInPatchCollide(traceWork_t *tw, const struct patchCollide_s *pc) { (void)tw; (void)pc; return qfalse; }

/* ---- tiny TSV helpers ---------------------------------------------------- */
typedef struct { char name[32]; char variant[32]; unsigned value; } enumrow_t;
static enumrow_t enums[1024];
static int numEnums;

static int split(char *line, char **cells, int max) {
  int n = 0; char *p = line;
  cells[n++] = p;
  while (*p && n < max) { if (*p == '\t') { *p = 0; cells[n++] = p + 1; } p++; }
  return n;
}
static void load_enums(const char *path) {
  FILE *f = fopen(path, "r"); char line[1024]; int header = 0;
  if (!f) { fprintf(stderr, "no %s\n", path); exit(2); }
  while (fgets(line, sizeof line, f)) {
    char *cells[8];
    line[strcspn(line, "\n")] = 0;
    if (line[0] == '#' || !line[0]) continue;
    if (!header) { header = 1; continue; }
    if (split(line, cells, 8) < 4) continue;
    strncpy(enums[numEnums].name, cells[0], 31);
    strncpy(enums[numEnums].variant, cells[2], 31);
    enums[numEnums].value = (unsigned)strtoul(cells[3], NULL, 10);
    numEnums++;
  }
  fclose(f);
}
static int flags_value(const char *en, const char *cell) {
  char buf[256]; unsigned v = 0; char *tok;
  if (!strcmp(cell, "NULL")) return 0;
  strncpy(buf, cell, 255); buf[255] = 0;
  for (tok = strtok(buf, "|"); tok; tok = strtok(NULL, "|")) {
    int i, found = 0;
    for (i = 0; i < numEnums; i++)
      if (!strcmp(enums[i].name, en) && !strcmp(enums[i].variant, tok)) { v |= enums[i].value; found = 1; }
    if (!found) { fprintf(stderr, "unknown %s variant %s\n", en, tok); exit(2); }
  }
  return (int)v;
}

/* static storage, generous for test maps */
static cplane_t planes[8192];
static cbrushside_t sides[8192];
static cbrush_t brushes[1024];
static cNode_t nodes[1];
static cLeaf_t leafs[1];
static int leafbrushes[1024];

static void push_plane(float nx, float ny, float nz, float dist, int type) {
  cplane_t *p = &planes[cm.numPlanes++];
  p->normal[0] = nx; p->normal[1] = ny; p->normal[2] = nz; p->dist = dist;
  p->type = type; SetPlaneSignbits(p);
}

void harness_load_world(const char *sheets_dir, const char *map) {
  char path[1024], line[4096];
  FILE *f; int header = 0;
  snprintf(path, sizeof path, "%s/01-enums.tsv", sheets_dir); load_enums(path);
  snprintf(path, sizeof path, "%s/domain/brushes.tsv", sheets_dir);
  f = fopen(path, "r");
  if (!f) { fprintf(stderr, "no %s\n", path); exit(2); }
  memset(&cm, 0, sizeof cm);
  cm.planes = planes; cm.brushsides = sides; cm.brushes = brushes;
  cm.nodes = nodes; cm.leafs = leafs; cm.leafbrushes = leafbrushes;
  while (fgets(line, sizeof line, f)) {
    char *c[16]; int n, axis, surf, first; cbrush_t *b; vec3_t mins, maxs;
    line[strcspn(line, "\n")] = 0;
    if (line[0] == '#' || !line[0] || !strncmp(line, "//", 2)) continue;
    if (!header) { header = 1; continue; }
    n = split(line, c, 16);
    if (n < 8 || strcmp(c[1], map)) continue;
    sscanf(c[2], "%f %f %f", &mins[0], &mins[1], &mins[2]);
    sscanf(c[3], "%f %f %f", &maxs[0], &maxs[1], &maxs[2]);
    surf = flags_value("Surface", c[6]);
    first = cm.numBrushSides;
    for (axis = 0; axis < 3; axis++) {
      float pn[3] = { 0, 0, 0 }, nn[3] = { 0, 0, 0 };
      pn[axis] = 1; nn[axis] = -1;
      push_plane(pn[0], pn[1], pn[2], maxs[axis], axis);
      sides[cm.numBrushSides].plane = &planes[cm.numPlanes - 1]; sides[cm.numBrushSides++].surfaceFlags = surf;
      push_plane(nn[0], nn[1], nn[2], -mins[axis], 3 + axis);
      sides[cm.numBrushSides].plane = &planes[cm.numPlanes - 1]; sides[cm.numBrushSides++].surfaceFlags = surf;
    }
    if (strcmp(c[4], "NULL")) {
      float cut[4];
      sscanf(c[4], "%f %f %f %f", &cut[0], &cut[1], &cut[2], &cut[3]);
      push_plane(cut[0], cut[1], cut[2], cut[3], 3);
      sides[cm.numBrushSides].plane = &planes[cm.numPlanes - 1]; sides[cm.numBrushSides++].surfaceFlags = surf;
    }
    b = &brushes[cm.numBrushes];
    b->contents = flags_value("Contents", c[5]);
    VectorCopy(mins, b->bounds[0]); VectorCopy(maxs, b->bounds[1]);
    b->sides = &sides[first]; b->numsides = cm.numBrushSides - first;
    leafbrushes[cm.numBrushes] = cm.numBrushes;
    cm.numBrushes++; cm.numLeafBrushes++;
  }
  fclose(f);
  push_plane(1, 0, 0, -65536.0f, 0);
  nodes[0].plane = &planes[cm.numPlanes - 1];
  nodes[0].children[0] = nodes[0].children[1] = -1;
  cm.numNodes = 1;
  leafs[0].cluster = 0; leafs[0].area = 0; leafs[0].firstLeafBrush = 0; leafs[0].numLeafBrushes = cm.numBrushes;
  cm.numLeafs = 1;
  world_model.leaf = leafs[0];
}
