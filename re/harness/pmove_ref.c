/* Golden-trace reference harness (MDD §7.2/§7.5): runs the ORIGINAL Pmove()
 * against the ORIGINAL cm_trace.c over a sheet-authored world, for every row
 * of sheets/domain/scenarios.tsv, and dumps (tick, entity, key, value) TSV.
 * Floats are dumped as IEEE-754 bit patterns so comparison is exact (dec008).
 *
 * usage: pmove_ref <sheets_dir> <map> [scenario_id]   > trace.tsv          */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "q_shared.h"
#include "bg_public.h"
#include "cm_local.h"

void harness_load_world(const char *sheets_dir, const char *map);
void harness_set_asset_root(const char *r);
void CM_BoxTrace(trace_t *results, const vec3_t start, const vec3_t end, vec3_t mins, vec3_t maxs,
                 clipHandle_t model, int brushmask, traceType_t type);
int CM_PointContents(const vec3_t p, clipHandle_t model);

static void h_trace(trace_t *results, const vec3_t start, const vec3_t mins, const vec3_t maxs,
                    const vec3_t end, int passEntityNum, int contentMask) {
  (void)passEntityNum;
  /* SV_Trace with no entities: world clip + entityNum convention */
  CM_BoxTrace(results, start, end, (float *)mins, (float *)maxs, 0, contentMask, TT_AABB);
  results->entityNum = results->fraction != 1.0 ? ENTITYNUM_WORLD : ENTITYNUM_NONE;
}
static int h_pointcontents(const vec3_t p, int pass) { (void)pass; return CM_PointContents(p, 0); }

static unsigned fb(float f) { unsigned u; memcpy(&u, &f, 4); return u; }
static void dv(int tick, const char *ent, const char *key, const float *v) {
  printf("%d\t%s\t%s\t%08x %08x %08x\n", tick, ent, key, fb(v[0]), fb(v[1]), fb(v[2]));
}
static void di(int tick, const char *ent, const char *key, int v) { printf("%d\t%s\t%s\t%d\n", tick, ent, key, v); }

static const char *pmtypes[] = { "Normal", "Noclip", "Spectator", "Jetpack", "Grabbed", "Dead", "Freeze", "Intermission" };
static int split(char *line, char **cells, int max) {
  int n = 0; char *p = line; cells[n++] = p;
  while (*p && n < max) { if (*p == '\t') { *p = 0; cells[n++] = p + 1; } p++; }
  return n;
}

static void run(char **c, int only) {
  const char *id = c[0];
  playerState_t ps; pmoveExt_t pmext; pmove_t pm; usercmd_t cmd;
  const classAttributes_t *ca = BG_ClassByName(c[1]);
  classConfig_t *cc = BG_ClassConfig(ca->number);
  int i, tick = 0, msec = atoi(c[8]);
  float yaw = (float)atof(c[5]);
  char script[2048], *tok;
  (void)only;
  memset(&ps, 0, sizeof ps); memset(&pmext, 0, sizeof pmext); memset(&cmd, 0, sizeof cmd);
  ps.stats[STAT_CLASS] = ca->number;
  ps.stats[STAT_TEAM] = ca->number >= PCL_HUMAN ? TEAM_HUMANS : ca->number == PCL_NONE ? TEAM_NONE : TEAM_ALIENS;
  ps.stats[STAT_HEALTH] = ps.stats[STAT_MAX_HEALTH] = ca->health ? ca->health : 100;
  ps.stats[STAT_STAMINA] = STAMINA_MAX;
  ps.speed = (int)(320.0f * ca->speed);
  ps.gravity = DEFAULT_GRAVITY;
  ps.viewheight = cc->viewheight;
  ps.groundEntityNum = ENTITYNUM_NONE;
  ps.grapplePoint[2] = 1.0f;
  ps.clientNum = 0;
  for (i = 0; i < 8; i++) if (!strcmp(c[6], pmtypes[i])) ps.pm_type = i;
  if (strcmp(c[7], "NULL")) {
    if (strstr(c[7], "Nonsegmodel")) ps.persistant[PERS_STATE] |= PS_NONSEGMODEL;
    if (strstr(c[7], "Wallclimbingtoggle")) ps.persistant[PERS_STATE] |= PS_WALLCLIMBINGTOGGLE;
    if (strstr(c[7], "Sprinttoggle")) ps.persistant[PERS_STATE] |= PS_SPRINTTOGGLE;
  }
  if (strcmp(c[2], "NULL")) {
    const weaponAttributes_t *wa = BG_WeaponByName(c[2]);
    ps.stats[STAT_WEAPON] = ps.weapon = wa->number;
    ps.ammo = wa->maxAmmo; ps.clips = wa->maxClips;
  }
  if (strcmp(c[3], "NULL")) {
    char items[256]; char *it; strncpy(items, c[3], 255); items[255] = 0;
    for (it = strtok(items, " "); it; it = strtok(NULL, " ")) BG_AddUpgradeToInventory(BG_UpgradeByName(it)->number, ps.stats);
  }
  sscanf(c[4], "%f %f %f", &ps.origin[0], &ps.origin[1], &ps.origin[2]);
  cmd.angles[YAW] = ANGLE2SHORT(yaw);

  strncpy(script, c[9], sizeof script - 1); script[sizeof script - 1] = 0;
  for (tok = strtok(script, " "); tok; tok = strtok(NULL, " ")) {
    int n = atoi(tok), f = 0, r = 0, u = 0, b = 0, y = 0, p = 0, w = 0, t;
    char *s = strchr(tok, '*') + 1;
    while (*s) {
      char k = *s++; int v = (int)strtol(s, &s, 10);
      switch (k) { case 'f': f = v; break; case 'r': r = v; break; case 'u': u = v; break; case 'b': b = v; break;
                   case 'y': y = v; break; case 'p': p = v; break; case 'w': w = v; break; default: exit(3); }
    }
    for (t = 0; t < n; t++) {
      tick++;
      cmd.serverTime += msec;
      cmd.forwardmove = f; cmd.rightmove = r; cmd.upmove = u; cmd.buttons = b; cmd.weapon = w;
      cmd.angles[YAW] += y; cmd.angles[PITCH] += p;
      memset(&pm, 0, sizeof pm);
      pm.ps = &ps; pm.pmext = &pmext; pm.cmd = cmd;
      pm.tracemask = ps.stats[STAT_HEALTH] > 0 ? MASK_PLAYERSOLID : MASK_DEADSOLID;
      pm.trace = h_trace; pm.pointcontents = h_pointcontents;
      pm.pmove_fixed = 0; pm.pmove_msec = 8;
      Pmove(&pm);
      dv(tick, id, "origin", ps.origin);
      dv(tick, id, "velocity", ps.velocity);
      dv(tick, id, "viewangles", ps.viewangles);
      dv(tick, id, "grapple", ps.grapplePoint);
      printf("%d\t%s\tdelta_angles\t%d %d %d\n", tick, id, ps.delta_angles[0], ps.delta_angles[1], ps.delta_angles[2]);
      di(tick, id, "pm_flags", ps.pm_flags); di(tick, id, "pm_time", ps.pm_time);
      di(tick, id, "ground", ps.groundEntityNum); di(tick, id, "eflags", ps.eFlags);
      di(tick, id, "stat_state", ps.stats[STAT_STATE]); di(tick, id, "stat_misc", ps.stats[STAT_MISC]);
      di(tick, id, "stamina", ps.stats[STAT_STAMINA]); di(tick, id, "falldist", ps.stats[STAT_FALLDIST]);
      di(tick, id, "weapon", ps.weapon); di(tick, id, "weapon_time", ps.weaponTime);
      di(tick, id, "weaponstate", ps.weaponstate); di(tick, id, "ammo", ps.ammo); di(tick, id, "clips", ps.clips);
      di(tick, id, "legs_anim", ps.legsAnim); di(tick, id, "torso_anim", ps.torsoAnim); di(tick, id, "weapon_anim", ps.weaponAnim);
      printf("%d\t%s\tevents\t%d %d %d\n", tick, id, ps.eventSequence, ps.events[0], ps.events[1]);
      di(tick, id, "bob_cycle", ps.bobCycle); di(tick, id, "viewheight", ps.viewheight);
      di(tick, id, "movement_dir", ps.movementDir); di(tick, id, "generic1", ps.generic1);
      di(tick, id, "pounce_payload", pmext.pouncePayload);
      di(tick, id, "touch", pm.numtouch); di(tick, id, "waterlevel", pm.waterlevel);
    }
  }
}

int main(int argc, char **argv) {
  char path[1024], line[4096]; FILE *f; int header = 0;
  if (argc < 3) { fprintf(stderr, "usage: pmove_ref <sheets_dir> <map> [scenario]\n"); return 2; }
  { static char root[1024]; snprintf(root, sizeof root, "%s/../re/source/assets", argv[1]); harness_set_asset_root(root); }
  BG_InitClassConfigs();
  harness_load_world(argv[1], argv[2]);
  snprintf(path, sizeof path, "%s/domain/scenarios.tsv", argv[1]);
  f = fopen(path, "r");
  if (!f) { fprintf(stderr, "no %s\n", path); return 2; }
  printf("tick\tentity\tkey\tvalue\n");
  while (fgets(line, sizeof line, f)) {
    char *c[16];
    line[strcspn(line, "\n")] = 0;
    if (line[0] == '#' || !line[0]) continue;
    if (!header) { header = 1; continue; }
    if (split(line, c, 16) < 10) continue;
    if (argc > 3 && strcmp(argv[3], c[0])) continue;
    srand(0x5eed1234u);
    run(c, argc > 3);
  }
  fclose(f);
  return 0;
}
