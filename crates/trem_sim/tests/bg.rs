//! bg_misc.c logic over the generated tables.
use trem_sim::gen::classes::ClassId;
use trem_sim::gen::constants as k;
use trem_sim::gen::enums::Traj;
use trem_sim::gen::upgrades::UpgradeId;
use trem_sim::gen::weapons::WeaponId;
use trem_sim::kernel::bg;

#[test]
fn inventory_bits_roundtrip() {
    let mut stats = [0i32; k::MAX_STATS as usize];
    let h = UpgradeId::Helmet.num() as i32;
    assert!(!bg::inventory_contains_upgrade(h, &stats));
    bg::add_upgrade(h, &mut stats);
    assert!(bg::inventory_contains_upgrade(h, &stats));
    bg::activate_upgrade(h, &mut stats);
    assert!(bg::upgrade_is_active(h, &stats));
    bg::deactivate_upgrade(h, &mut stats);
    assert!(!bg::upgrade_is_active(h, &stats));
    bg::remove_upgrade(h, &mut stats);
    assert!(!bg::inventory_contains_upgrade(h, &stats));
}

#[test]
fn evolve_costs_follow_the_class_graph() {
    let per = k::ALIEN_CREDITS_PER_KILL;
    let l1 = ClassId::Level1.def().cost * per;
    assert_eq!(bg::class_can_evolve_from_to(ClassId::Level0, ClassId::Level1, l1, 0, 0), l1);
    assert_eq!(bg::class_can_evolve_from_to(ClassId::Level0, ClassId::Level1, l1 - 1, 0, 0), -1);
    // level4 is stage 3 only
    assert_eq!(bg::class_can_evolve_from_to(ClassId::Level0, ClassId::Level4, 1_000_000, 0, 0), -1);
    assert!(bg::class_can_evolve_from_to(ClassId::Level0, ClassId::Level4, 1_000_000, 2, 0) > 0);
    assert!(bg::alien_can_evolve(ClassId::Level0, l1, 0));
}

#[test]
fn weapon_lookup_has_null_weapon() {
    assert_eq!(bg::weapon(0).max_ammo, 0);
    assert_eq!(bg::weapon(WeaponId::Rifle as i32).id, "rifle");
}

#[test]
fn gravity_trajectory_is_a_parabola() {
    let tr = bg::Trajectory { tr_type: Traj::Gravity as i32, tr_time: 0, tr_duration: 0, tr_base: [0.0, 0.0, 100.0], tr_delta: [10.0, 0.0, 0.0] };
    let p = bg::evaluate_trajectory(&tr, 1000);
    assert!((p[0] - 10.0).abs() < 1e-3);
    assert!((p[2] - (100.0 - 0.5 * k::DEFAULT_GRAVITY as f32)).abs() < 1e-2);
    let v = bg::evaluate_trajectory_delta(&tr, 1000);
    assert!((v[2] + k::DEFAULT_GRAVITY as f32).abs() < 1e-2);
}
