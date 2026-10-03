/* Dumps the ORIGINAL bg_misc.c attribute tables (after config parsing) as raw
 * TSV. tools: re/harness/synth_sheets.py turns this into canonical sheets.
 * Numeric enums are dumped as integers; the synthesizer maps them to ids. */
#include <stdio.h>
#include "q_shared.h"
#include "bg_public.h"

void harness_set_asset_root(const char *r);

static void v3(const vec3_t v) { printf("%.9g %.9g %.9g", v[0], v[1], v[2]); }
static const char *s(const char *x) { return x ? x : ""; }

int main(int argc, char **argv) {
  int i;
  if (argc > 1) harness_set_asset_root(argv[1]);
  BG_InitClassConfigs();
  BG_InitBuildableConfigs();

  printf("@classes\n");
  printf("num\tname\tstages\thealth\tfall_damage\tregen_rate\tabilities\tstart_weapon\tbuild_dist\tfov\tbob\tbob_cycle\tsteptime\tspeed\tacceleration\tair_acceleration\tfriction\tstop_speed\tjump_magnitude\tknockback_scale\tchild0\tchild1\tchild2\tcost\tvalue"
         "\tmodel\tmodel_scale\tskin\tshadow_scale\thud\thuman_name\tmins\tmaxs\tcrouch_maxs\tdead_mins\tdead_maxs\tviewheight\tcrouch_viewheight\tz_offset\tshoulder_offsets\n");
  for (i = 0; i < PCL_NUM_CLASSES; i++) {
    const classAttributes_t *c = BG_Class(i);
    classConfig_t *cc = BG_ClassConfig(i);
    printf("%d\t%s\t%d\t%d\t%.9g\t%.9g\t%d\t%d\t%.9g\t%d\t%.9g\t%.9g\t%d\t%.9g\t%.9g\t%.9g\t%.9g\t%.9g\t%.9g\t%.9g\t%d\t%d\t%d\t%d\t%d",
           c->number, s(c->name), c->stages, c->health, c->fallDamage, c->regenRate, c->abilities,
           c->startWeapon, c->buildDist, c->fov, c->bob, c->bobCycle, c->steptime, c->speed,
           c->acceleration, c->airAcceleration, c->friction, c->stopSpeed, c->jumpMagnitude,
           c->knockbackScale, c->children[0], c->children[1], c->children[2], c->cost, c->value);
    printf("\t%s\t%.9g\t%s\t%.9g\t%s\t%s\t", cc->modelName, cc->modelScale, cc->skinName,
           cc->shadowScale, cc->hudName, cc->humanName);
    v3(cc->mins); printf("\t"); v3(cc->maxs); printf("\t"); v3(cc->crouchMaxs); printf("\t");
    v3(cc->deadMins); printf("\t"); v3(cc->deadMaxs);
    printf("\t%d\t%d\t%.9g\t", cc->viewheight, cc->crouchViewheight, cc->zOffset);
    v3(cc->shoulderOffsets); printf("\n");
  }

  printf("@buildables\n");
  printf("num\tname\thuman_name\tentity_name\ttraj\tbounce\tbuild_points\tstages\thealth\tregen_rate\tsplash_damage\tsplash_radius\tmeans_of_death\tteam\tbuild_weapon\tidle_anim\tnextthink\tbuild_time\tusable\tturret_range\tturret_fire_speed\tturret_proj\tmin_normal\tinvert_normal\tcreep_test\tcreep_size\tdcc_test\ttransparent_test\tunique_test\tvalue"
         "\tmodel0\tmodel_scale\tmins\tmaxs\tz_offset\n");
  for (i = BA_NONE + 1; i < BA_NUM_BUILDABLES; i++) {
    const buildableAttributes_t *b = BG_Buildable(i);
    buildableConfig_t *bc = BG_BuildableConfig(i);
    printf("%d\t%s\t%s\t%s\t%d\t%.9g\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%.9g\t%d\t%d\t%d\t%d\t%d\t%d\t%d",
           b->number, s(b->name), s(b->humanName), s(b->entityName), b->traj, b->bounce,
           b->buildPoints, b->stages, b->health, b->regenRate, b->splashDamage, b->splashRadius,
           b->meansOfDeath, b->team, b->buildWeapon, b->idleAnim, b->nextthink, b->buildTime,
           b->usable, b->turretRange, b->turretFireSpeed, b->turretProjType, b->minNormal,
           b->invertNormal, b->creepTest, b->creepSize, b->dccTest, b->transparentTest,
           b->uniqueTest, b->value);
    printf("\t%s\t%.9g\t", bc->models[0], bc->modelScale);
    v3(bc->mins); printf("\t"); v3(bc->maxs); printf("\t%.9g\n", bc->zOffset);
  }

  printf("@weapons\n");
  printf("num\tname\thuman_name\tprice\tstages\tslots\tmax_ammo\tmax_clips\tinfinite_ammo\tuses_energy\trepeat_rate1\trepeat_rate2\trepeat_rate3\treload_time\tknockback_scale\thas_alt_mode\thas_third_mode\tcan_zoom\tzoom_fov\tpurchasable\tlong_ranged\tteam\n");
  for (i = WP_NONE + 1; i < WP_NUM_WEAPONS; i++) {
    const weaponAttributes_t *w = BG_Weapon(i);
    printf("%d\t%s\t%s\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%.9g\t%d\t%d\t%d\t%.9g\t%d\t%d\t%d\n",
           w->number, s(w->name), s(w->humanName), w->price, w->stages, w->slots, w->maxAmmo,
           w->maxClips, w->infiniteAmmo, w->usesEnergy, w->repeatRate1, w->repeatRate2,
           w->repeatRate3, w->reloadTime, w->knockbackScale, w->hasAltMode, w->hasThirdMode,
           w->canZoom, w->zoomFov, w->purchasable, w->longRanged, w->team);
  }

  printf("@upgrades\n");
  printf("num\tname\thuman_name\tprice\tstages\tslots\ticon\tpurchasable\tusable\tteam\n");
  for (i = UP_NONE + 1; i < UP_NUM_UPGRADES; i++) {
    const upgradeAttributes_t *u = BG_Upgrade(i);
    printf("%d\t%s\t%s\t%d\t%d\t%d\t%s\t%d\t%d\t%d\n", u->number, s(u->name), s(u->humanName),
           u->price, u->stages, u->slots, s(u->icon), u->purchasable, u->usable, u->team);
  }
  return 0;
}
