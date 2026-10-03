/* Curve oracle: the ORIGINAL renderergl1/tr_curve.c R_SubdividePatchToGrid
 * (built against curve/tr_local.h, a stub with only its types) over every
 * patch surface of a BSP. Prints, per patch surface: "surface width height"
 * and every grid vertex's xyz and normal as float bits, which
 * crates/trem_sim/tests/marks.rs hashes against re/traces/curve/<map>.sha256.
 * usage: curve_ref file.bsp */
#include "tr_local.h"
#include <stdio.h>
#include <string.h>
#include <stdarg.h>

static curve_cvar_t subdivisions = { 4.0f };
curve_cvar_t *r_subdivisions = &subdivisions;
static void *m_alloc(int n) { return calloc(1, n); }
static void *h_alloc(int n, int pref) { (void)pref; return calloc(1, n); }
curve_refimport_t ri = { m_alloc, free, h_alloc };

srfGridMesh_t *R_SubdividePatchToGrid( int width, int height, drawVert_t points[MAX_PATCH_SIZE*MAX_PATCH_SIZE] );

void QDECL Com_Error(int level, const char *fmt, ...) { (void)level; fprintf(stderr, "%s\n", fmt); exit(1); }
void QDECL Com_Printf(const char *fmt, ...) { (void)fmt; }

static unsigned bits(float f) { unsigned u; memcpy(&u, &f, 4); return u; }

int main(int argc, char **argv) {
	FILE *f;
	long n;
	unsigned char *b;
	dheader_t *h;
	dsurface_t *surfs;
	drawVert_t *verts;
	int ns, i, k;
	if (argc != 2 || !(f = fopen(argv[1], "rb"))) { fprintf(stderr, "usage: curve_ref file.bsp\n"); return 1; }
	fseek(f, 0, SEEK_END); n = ftell(f); fseek(f, 0, SEEK_SET);
	b = malloc(n);
	if (fread(b, 1, n, f) != (size_t)n) return 1;
	fclose(f);
	h = (dheader_t *)b;
	surfs = (dsurface_t *)(b + h->lumps[LUMP_SURFACES].fileofs);
	ns = h->lumps[LUMP_SURFACES].filelen / sizeof(dsurface_t);
	verts = (drawVert_t *)(b + h->lumps[LUMP_DRAWVERTS].fileofs);
	for (i = 0; i < ns; i++) {
		drawVert_t points[MAX_PATCH_SIZE * MAX_PATCH_SIZE];
		srfGridMesh_t *g;
		dsurface_t *s = &surfs[i];
		if (s->surfaceType != MST_PATCH || s->patchWidth > MAX_PATCH_SIZE || s->patchHeight > MAX_PATCH_SIZE)
			continue;
		memcpy(points, verts + s->firstVert, s->numVerts * sizeof(drawVert_t));
		g = R_SubdividePatchToGrid(s->patchWidth, s->patchHeight, points);
		printf("%d %d %d\n", i, g->width, g->height);
		for (k = 0; k < g->width * g->height; k++) {
			drawVert_t *v = &g->verts[k];
			printf("%08x %08x %08x %08x %08x %08x\n", bits(v->xyz[0]), bits(v->xyz[1]), bits(v->xyz[2]), bits(v->normal[0]), bits(v->normal[1]), bits(v->normal[2]));
		}
	}
	return 0;
}
