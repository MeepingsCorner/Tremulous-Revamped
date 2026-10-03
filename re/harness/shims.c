/* Engine shims so the ORIGINAL Tremulous bg_*.c / q_*.c / cm_*.c compile as a
 * native, deterministic reference oracle (MDD §7.2). Not shipped. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <math.h>
#include "q_shared.h"

static const char *asset_root = "../source/assets";
void harness_set_asset_root(const char *r) { asset_root = r; }

static FILE *files[64];

int trap_FS_FOpenFile(const char *qpath, fileHandle_t *f, fsMode_t mode) {
  char p[1024];
  (void)mode;
  snprintf(p, sizeof p, "%s/%s", asset_root, qpath);
  FILE *fp = fopen(p, "rb");
  if (!fp) {
    /* a missing class/buildable config silently zeroes the oracle: refuse */
    if (!strncmp(qpath, "configs/", 8)) { fprintf(stderr, "harness: missing asset %s\n", p); exit(3); }
    *f = 0; return -1;
  }
  for (int i = 1; i < 64; i++) if (!files[i]) {
    files[i] = fp; *f = i;
    fseek(fp, 0, SEEK_END); long n = ftell(fp); fseek(fp, 0, SEEK_SET);
    return (int)n;
  }
  fclose(fp); *f = 0; return -1;
}
void trap_FS_Read(void *buffer, int len, fileHandle_t f) { if (files[f]) fread(buffer, 1, len, files[f]); }
void trap_FS_Write(const void *b, int l, fileHandle_t f) { (void)b; (void)l; (void)f; }
void trap_FS_FCloseFile(fileHandle_t f) { if (files[f]) { fclose(files[f]); files[f] = NULL; } }
void trap_FS_Seek(fileHandle_t f, long o, int w) { (void)f; (void)o; (void)w; }
int trap_FS_GetFileList(const char *a, const char *b, char *c, int d) { (void)a; (void)b; (void)c; (void)d; return 0; }
void trap_Cvar_VariableStringBuffer(const char *n, char *buf, int sz) { (void)n; if (sz) buf[0] = 0; }

/* x86_64 engine uses qsnapvectorsse: cvtss2si, round-half-to-even under default MXCSR. */
void trap_SnapVector(float *v) { v[0] = rintf(v[0]); v[1] = rintf(v[1]); v[2] = rintf(v[2]); }

void QDECL Com_Printf(const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); vfprintf(stderr, fmt, ap); va_end(ap);
}
void QDECL Com_Error(int level, const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); vfprintf(stderr, fmt, ap); va_end(ap);
  (void)level; exit(2);
}

/* dec013: deterministic rand() shared with kernel rng.rs (overrides libc). */
static unsigned int harness_rand_state = 0x5eed1234u;
int rand(void) {
  harness_rand_state = harness_rand_state * 1103515245u + 12345u;
  return (int)((harness_rand_state >> 1) & 0x7fffffffu);
}
void srand(unsigned int s) { harness_rand_state = s; }
