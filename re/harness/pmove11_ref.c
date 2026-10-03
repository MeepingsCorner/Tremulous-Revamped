/* Tremulous 1.1.0 golden-trace oracle (dec041): runs the ORIGINAL 1.1.0
 * Pmove() (re/source11 bg_pmove.c / bg_slidemove.c / bg_misc.c with
 * data-1.1.0.pk3's overrides) against the shared collision (the legacy
 * cm_trace.c over the sheet world, like pmove_ref.c), for every row of
 * sheets/domain/scenarios11.tsv, and dumps (tick, entity, key, value) TSV.
 * Floats are dumped as IEEE-754 bit patterns (dec008).
 *
 * The player is set up like 1.1.0's ClientSpawn (g_client.c): humans also
 * carry the blaster, the weapon gets its full ammo packed in ps.ammo[] /
 * ps.powerups[], ps.speed = 320 * BG_FindSpeedForClass. Script tokens are
 * pmove_ref.c's plus the server-side state pmove reads but never writes:
 * m<int> sets STAT_MISC (the pounce / charge / lcannon charge g_active.c
 * builds up), s<int> ORs into STAT_STATE (e.g. SS_SPEEDBOOST from "boost",
 * SS_CHARGING), c<int> clears STAT_STATE bits; each applies before every tick
 * of its token.
 *
 * usage: pmove11_ref <sheets_dir> <map> [scenario_id]   > trace.tsv        */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "q_shared.h"
#include "bg_public.h"

void harness_load_world(const char *sheets_dir, const char *map);
void harness_set_asset_root(const char *r);
void CM_BoxTrace(trace_t *results, const vec3_t start, const vec3_t end, vec3_t mins, vec3_t maxs,
                 clipHandle_t model, int brushmask, int type);
int CM_PointContents(const vec3_t p, clipHandle_t model);

static void h_trace(trace_t *results, const vec3_t start, const vec3_t mins, const vec3_t maxs,
                    const vec3_t end, int passEntityNum, int contentMask) {
  (void)passEntityNum;
  CM_BoxTrace(results, start, end, (float *)mins, (float *)maxs, 0, contentMask, 1 /* legacy TT_AABB */);
  results->entityNum = results->fraction != 1.0 ? ENTITYNUM_WORLD : ENTITYNUM_NONE;
}
static int h_pointcontents(const vec3_t p, int pass) { (void)pass; return CM_PointContents(p, 0); }

static unsigned fb(float f) { unsigned u; memcpy(&u, &f, 4); return u; }
static void dv(int tick, const char *ent, const char *key, const float *v) {
  printf("%d\t%s\t%s\t%08x %08x %08x\n", tick, ent, key, fb(v[0]), fb(v[1]), fb(v[2]));
}
static void di(int tick, const char *ent, const char *key, int v) { printf("%d\t%s\t%s\t%d\n", tick, ent, key, v); }

static const char *pmtypes[] = { "Normal", "Noclip", "Spectator", "Jetpack", "Grabbed", "Dead", "Freeze", "Intermission", "Spintermission" };
static int split(char *line, char **cells, int max) {
  int n = 0; char *p = line; cells[n++] = p;
  while (*p && n < max) { if (*p == '\t') { *p = 0; cells[n++] = p + 1; } p++; }
  return n;
}

static void run(char **c) {
  const char *id = c[0];
  playerState_t ps; pmove_t pm; usercmd_t cmd;
  int pclass = BG_FindClassNumForName(c[1]);
  int i, tick = 0, msec = atoi(c[8]), vh, cvh, health;
  float yaw = (float)atof(c[5]);
  char script[2048], *tok;
  memset(&ps, 0, sizeof ps); memset(&cmd, 0, sizeof cmd);
  ps.stats[STAT_PCLASS] = pclass;
  ps.stats[STAT_PTEAM] = pclass >= PCL_HUMAN ? PTE_HUMANS : pclass == PCL_NONE ? PTE_NONE : PTE_ALIENS;
  ps.persistant[PERS_TEAM] = pclass == PCL_NONE ? TEAM_SPECTATOR : TEAM_FREE;
  health = BG_FindHealthForClass(pclass);
  ps.stats[STAT_HEALTH] = ps.stats[STAT_MAX_HEALTH] = health ? health : 100;
  ps.stats[STAT_STAMINA] = MAX_STAMINA;
  ps.speed = (int)(320.0f * BG_FindSpeedForClass(pclass));
  ps.gravity = DEFAULT_GRAVITY;
  BG_FindViewheightForClass(pclass, &vh, &cvh);
  ps.viewheight = vh;
  ps.groundEntityNum = ENTITYNUM_NONE;
  ps.grapplePoint[2] = 1.0f;
  ps.clientNum = 0;
  for (i = 0; i < 9; i++) if (!strcmp(c[6], pmtypes[i])) ps.pm_type = i;
  if (strcmp(c[7], "NULL")) {
    if (strstr(c[7], "Nonsegmodel")) ps.persistant[PERS_STATE] |= PS_NONSEGMODEL;
    if (strstr(c[7], "Wallclimbingtoggle")) ps.persistant[PERS_STATE] |= PS_WALLCLIMBINGTOGGLE;
    if (strstr(c[7], "Wallclimbingfollow")) ps.persistant[PERS_STATE] |= PS_WALLCLIMBINGFOLLOW;
  }
  if (ps.stats[STAT_PTEAM] == PTE_HUMANS) BG_AddWeaponToInventory(WP_BLASTER, ps.stats);
  if (strcmp(c[2], "NULL")) {
    int w = BG_FindWeaponNumForName(c[2]), ma = 0, mc = 0;
    BG_AddWeaponToInventory(w, ps.stats);
    ps.weapon = w;
    BG_FindAmmoForWeapon(w, &ma, &mc);
    BG_PackAmmoArray(w, ps.ammo, ps.powerups, ma, mc);
  }
  if (strcmp(c[3], "NULL")) {
    char items[256]; char *it; strncpy(items, c[3], 255); items[255] = 0;
    for (it = strtok(items, " "); it; it = strtok(NULL, " ")) BG_AddUpgradeToInventory(BG_FindUpgradeNumForName(it), ps.stats);
  }
  sscanf(c[4], "%f %f %f", &ps.origin[0], &ps.origin[1], &ps.origin[2]);
  cmd.angles[YAW] = ANGLE2SHORT(yaw);

  strncpy(script, c[9], sizeof script - 1); script[sizeof script - 1] = 0;
  for (tok = strtok(script, " "); tok; tok = strtok(NULL, " ")) {
    int n = atoi(tok), f = 0, r = 0, u = 0, b = 0, y = 0, p = 0, w = 0, t;
    int setmisc = 0, misc = 0, sor = 0, sclr = 0;
    char *s = strchr(tok, '*') + 1;
    while (*s) {
      char k = *s++; int v = (int)strtol(s, &s, 10);
      switch (k) { case 'f': f = v; break; case 'r': r = v; break; case 'u': u = v; break; case 'b': b = v; break;
                   case 'y': y = v; break; case 'p': p = v; break; case 'w': w = v; break;
                   case 'm': setmisc = 1; misc = v; break; case 's': sor = v; break; case 'c': sclr = v; break;
                   default: exit(3); }
    }
    for (t = 0; t < n; t++) {
      int a, cl;
      tick++;
      if (setmisc) ps.stats[STAT_MISC] = misc;
      ps.stats[STAT_STATE] |= sor;
      ps.stats[STAT_STATE] &= ~sclr;
      cmd.serverTime += msec;
      cmd.forwardmove = f; cmd.rightmove = r; cmd.upmove = u; cmd.buttons = b; cmd.weapon = w;
      cmd.angles[YAW] += y; cmd.angles[PITCH] += p;
      memset(&pm, 0, sizeof pm);
      pm.ps = &ps; pm.cmd = cmd;
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
      di(tick, id, "activeitems", ps.stats[STAT_ACTIVEITEMS]); di(tick, id, "buildable", ps.stats[STAT_BUILDABLE]);
      di(tick, id, "weapon", ps.weapon); di(tick, id, "weapon_time", ps.weaponTime);
      di(tick, id, "weaponstate", ps.weaponstate);
      BG_UnpackAmmoArray(ps.weapon, ps.ammo, ps.powerups, &a, &cl);
      di(tick, id, "ammo", a); di(tick, id, "clips", cl); di(tick, id, "newweapon", ps.persistant[PERS_NEWWEAPON]);
      di(tick, id, "legs_anim", ps.legsAnim); di(tick, id, "torso_anim", ps.torsoAnim);
      di(tick, id, "legs_timer", ps.legsTimer); di(tick, id, "torso_timer", ps.torsoTimer);
      printf("%d\t%s\tevents\t%d %d %d\n", tick, id, ps.eventSequence, ps.events[0], ps.events[1]);
      di(tick, id, "bob_cycle", ps.bobCycle); di(tick, id, "viewheight", ps.viewheight);
      di(tick, id, "movement_dir", ps.movementDir); di(tick, id, "generic1", ps.generic1);
      di(tick, id, "touch", pm.numtouch); di(tick, id, "waterlevel", pm.waterlevel);
    }
  }
}

int main(int argc, char **argv) {
  char path[1024], line[4096]; FILE *f; int header = 0;
  if (argc < 3) { fprintf(stderr, "usage: pmove11_ref <sheets_dir> <map> [scenario]\n"); return 2; }
  { static char root[1024]; snprintf(root, sizeof root, "%s/../re/source11/assets", argv[1]); harness_set_asset_root(root); }
  BG_InitClassOverrides();
  harness_load_world(argv[1], argv[2]);
  snprintf(path, sizeof path, "%s/domain/scenarios11.tsv", argv[1]);
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
    run(c);
  }
  fclose(f);
  return 0;
}
