/* Tremulous 1.1.0 attribute tables as the ORIGINAL 1.1.0 bg_misc.c serves
 * them (BG_Find* after BG_InitClassOverrides / BG_InitBuildableOverrides with
 * data-1.1.0.pk3's overrides/, kept in re/source11/assets), as raw TSV:
 * re/traces/v11/tables.tsv, the source of the classes11 / weapons11 /
 * upgrades11 / buildables11 sheets (re/harness/synth_v11.py, dec041).
 *
 * usage: v11_ref <asset_root> > tables.tsv                                  */
#include <stdio.h>
#include "q_shared.h"
#include "bg_public.h"

void harness_set_asset_root(const char *r);
extern classAttributes_t bg_classList[]; extern int bg_numPclasses;
extern buildableAttributes_t bg_buildableList[]; extern int bg_numBuildables;
extern weaponAttributes_t bg_weapons[]; extern int bg_numWeapons;
extern upgradeAttributes_t bg_upgrades[]; extern int bg_numUpgrades;

static void v3(const vec3_t v) { printf("%.9g %.9g %.9g", v[0], v[1], v[2]); }
static const char *s(const char *x) { return x && *x ? x : "NULL"; }
static int stages(qboolean (*f)(int, stage_t), int n) {
  return (f(n, S1) ? 1 : 0) | (f(n, S2) ? 2 : 0) | (f(n, S3) ? 4 : 0);
}

int main(int argc, char **argv) {
  int i, j;
  if (argc > 1) harness_set_asset_root(argv[1]);
  BG_InitClassOverrides();
  BG_InitBuildableOverrides();

  printf("@classes\n");
  printf("num\tname\thuman_name\tmodel\tmodel_scale\tskin\tshadow_scale\thud\tstages\tmins\tmaxs\tcrouch_maxs\tdead_mins\tdead_maxs\tz_offset"
         "\tviewheight\tcrouch_viewheight\thealth\tfall_damage\tregen_rate\tabilities\tstart_weapon\tbuild_dist\tfov\tbob\tbob_cycle\tsteptime"
         "\tspeed\tacceleration\tair_acceleration\tfriction\tstop_speed\tjump_magnitude\tknockback_scale\tchild0\tchild1\tchild2\tcost\tvalue\n");
  for (i = 0; i < PCL_NUM_CLASSES; i++) {
    const classAttributes_t *c = NULL; vec3_t mins, maxs, cmaxs, dmins, dmaxs; int vh, cvh;
    for (j = 0; j < bg_numPclasses; j++) if (bg_classList[j].classNum == i) c = &bg_classList[j];
    BG_FindBBoxForClass(i, mins, maxs, cmaxs, dmins, dmaxs);
    BG_FindViewheightForClass(i, &vh, &cvh);
    printf("%d\t%s\t%s\t%s\t%.9g\t%s\t%.9g\t%s\t%d\t", i, BG_FindNameForClassNum(i), BG_FindHumanNameForClassNum(i),
           s(BG_FindModelNameForClass(i)), BG_FindModelScaleForClass(i), s(BG_FindSkinNameForClass(i)),
           BG_FindShadowScaleForClass(i), s(BG_FindHudNameForClass(i)), stages(BG_FindStagesForClass, i));
    v3(mins); printf("\t"); v3(maxs); printf("\t"); v3(cmaxs); printf("\t"); v3(dmins); printf("\t"); v3(dmaxs);
    printf("\t%.9g\t%d\t%d\t%d\t%.9g\t%d\t%d\t%d\t%.9g\t%d\t%.9g\t%.9g\t%d\t%.9g\t%.9g\t%.9g\t%.9g\t%.9g\t%.9g\t%.9g\t%d\t%d\t%d\t%d\t%d\n",
           BG_FindZOffsetForClass(i), vh, cvh, BG_FindHealthForClass(i), BG_FindFallDamageForClass(i),
           BG_FindRegenRateForClass(i), c->abilities, BG_FindStartWeaponForClass(i), BG_FindBuildDistForClass(i),
           BG_FindFovForClass(i), BG_FindBobForClass(i), BG_FindBobCycleForClass(i), BG_FindSteptimeForClass(i),
           BG_FindSpeedForClass(i), BG_FindAccelerationForClass(i), BG_FindAirAccelerationForClass(i),
           BG_FindFrictionForClass(i), BG_FindStopSpeedForClass(i), BG_FindJumpMagnitudeForClass(i),
           BG_FindKnockbackScaleForClass(i), c->children[0], c->children[1], c->children[2],
           BG_FindCostOfClass(i), BG_FindValueOfClass(i));
  }

  printf("@buildables\n");
  printf("num\tname\thuman_name\tentity_name\tmodel0\tmodel1\tmodel2\tmodel_scale\tmins\tmaxs\tz_offset\ttraj\tbounce\tbuild_points\tstages"
         "\thealth\tregen_rate\tsplash_damage\tsplash_radius\tmeans_of_death\tteam\tbuild_weapon\tidle_anim\tnextthink\tbuild_time\tusable"
         "\tturret_range\tturret_fire_speed\tturret_proj\tmin_normal\tinvert_normal\tcreep_test\tcreep_size\tdcc_test\treactor_test\n");
  for (i = BA_NONE + 1; i < BA_NUM_BUILDABLES; i++) {
    vec3_t mins, maxs;
    BG_FindBBoxForBuildable(i, mins, maxs);
    printf("%d\t%s\t%s\t%s\t%s\t%s\t%s\t%.9g\t", i, BG_FindNameForBuildable(i), BG_FindHumanNameForBuildable(i),
           BG_FindEntityNameForBuildable(i), s(BG_FindModelsForBuildable(i, 0)), s(BG_FindModelsForBuildable(i, 1)),
           s(BG_FindModelsForBuildable(i, 2)), BG_FindModelScaleForBuildable(i));
    v3(mins); printf("\t"); v3(maxs);
    printf("\t%.9g\t%d\t%.9g\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%.9g\t%d\t%d\t%d\t%d\t%d\n",
           BG_FindZOffsetForBuildable(i), BG_FindTrajectoryForBuildable(i), BG_FindBounceForBuildable(i),
           BG_FindBuildPointsForBuildable(i), stages(BG_FindStagesForBuildable, i), BG_FindHealthForBuildable(i),
           BG_FindRegenRateForBuildable(i), BG_FindSplashDamageForBuildable(i), BG_FindSplashRadiusForBuildable(i),
           BG_FindMODForBuildable(i), BG_FindTeamForBuildable(i), BG_FindBuildWeaponForBuildable(i),
           BG_FindAnimForBuildable(i), BG_FindNextThinkForBuildable(i), BG_FindBuildTimeForBuildable(i),
           BG_FindUsableForBuildable(i), BG_FindRangeForBuildable(i), BG_FindFireSpeedForBuildable(i),
           BG_FindProjTypeForBuildable(i), BG_FindMinNormalForBuildable(i), BG_FindInvertNormalForBuildable(i),
           BG_FindCreepTestForBuildable(i), BG_FindCreepSizeForBuildable(i), BG_FindDCCTestForBuildable(i),
           BG_FindUniqueTestForBuildable(i));
  }

  printf("@weapons\n");
  printf("num\tname\thuman_name\tprice\tstages\tslots\tmax_ammo\tmax_clips\tinfinite_ammo\tuses_energy\trepeat_rate1\trepeat_rate2\trepeat_rate3"
         "\treload_time\thas_alt_mode\thas_third_mode\tcan_zoom\tzoom_fov\tpurchasable\tbuild_delay\tteam\n");
  for (i = WP_NONE + 1; i < WP_NUM_WEAPONS; i++) {
    int ma, mc;
    BG_FindAmmoForWeapon(i, &ma, &mc);
    printf("%d\t%s\t%s\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%.9g\t%d\t%d\t%d\n", i,
           BG_FindNameForWeapon(i), BG_FindHumanNameForWeapon(i), BG_FindPriceForWeapon(i), stages(BG_FindStagesForWeapon, i),
           BG_FindSlotsForWeapon(i), ma, mc, BG_FindInfinteAmmoForWeapon(i), BG_FindUsesEnergyForWeapon(i),
           BG_FindRepeatRate1ForWeapon(i), BG_FindRepeatRate2ForWeapon(i), BG_FindRepeatRate3ForWeapon(i),
           BG_FindReloadTimeForWeapon(i), BG_WeaponHasAltMode(i), BG_WeaponHasThirdMode(i), BG_WeaponCanZoom(i),
           BG_FindZoomFovForWeapon(i), BG_FindPurchasableForWeapon(i), BG_FindBuildDelayForWeapon(i), BG_FindTeamForWeapon(i));
  }

  printf("@upgrades\n");
  printf("num\tname\thuman_name\tprice\tstages\tslots\ticon\tpurchasable\tusable\tteam\n");
  for (i = UP_NONE + 1; i < UP_NUM_UPGRADES; i++)
    printf("%d\t%s\t%s\t%d\t%d\t%d\t%s\t%d\t%d\t%d\n", i, BG_FindNameForUpgrade(i), BG_FindHumanNameForUpgrade(i),
           BG_FindPriceForUpgrade(i), stages(BG_FindStagesForUpgrade, i), BG_FindSlotsForUpgrade(i),
           s(BG_FindIconForUpgrade(i)), BG_FindPurchasableForUpgrade(i), BG_FindUsableForUpgrade(i), BG_FindTeamForUpgrade(i));
  return 0;
}
