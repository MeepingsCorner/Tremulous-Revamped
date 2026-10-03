/* Stub tr_local.h for the curve oracle: just what renderergl1/tr_curve.c
 * uses (drawVert_t from qfiles.h, srfGridMesh_t, r_subdivisions, ri). */
#ifndef TR_LOCAL_STUB_H
#define TR_LOCAL_STUB_H
#include "q_shared.h"
#include "qfiles.h"
#include <stdlib.h>
#define MAX_PATCH_SIZE 32
#define MAX_GRID_SIZE 65
typedef enum { SF_BAD, SF_GRID = 4 } surfaceType_t;
typedef struct srfGridMesh_s {
	surfaceType_t surfaceType;
	int dlightBits;
	vec3_t meshBounds[2];
	vec3_t localOrigin;
	float meshRadius;
	vec3_t lodOrigin;
	float lodRadius;
	int lodFixed;
	int lodStitched;
	int width, height;
	float *widthLodError;
	float *heightLodError;
	drawVert_t verts[1];
} srfGridMesh_t;
typedef struct { float value; } curve_cvar_t;
extern curve_cvar_t *r_subdivisions;
typedef struct {
	void *(*Malloc)(int bytes);
	void (*Free)(void *p);
	void *(*Hunk_Alloc)(int size, int pref);
} curve_refimport_t;
extern curve_refimport_t ri;
#define Hunk_Alloc(size) Hunk_Alloc(size, 0)
#endif
