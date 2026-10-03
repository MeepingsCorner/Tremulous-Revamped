use trem_sim::gen::weapons11::Weapon11Id;
use trem_sim::kernel::bg11;
use trem_sim::kernel::v11::{ps_from_11, ps_to_11, PlayerState11};

#[test]
fn ammo_round_trip() {
    let mut p = PlayerState11::default();
    let rifle = Weapon11Id::Rifle.num() as i32;
    let blaster = Weapon11Id::Blaster.num() as i32;
    p.weapon = rifle;
    // weapons held: blaster + rifle (STAT_WEAPONS bit per weapon)
    p.stats[trem_sim::gen::enums::Stat11::Weapons as usize] = (1 << rifle) | (1 << blaster);
    bg11::pack_ammo_array(rifle, &mut p.ammo, &mut p.powerups, 30, 6);
    println!("rifle={rifle} blaster={blaster} ammo={:?} pw={:?} stats={:?}", p.ammo, p.powerups, p.stats);
    let s = ps_from_11(&p);
    println!("legacy ammo={} clips={} misc={:?}", s.ammo, s.clips, s.misc);
    let q = ps_to_11(&s);
    let (a, c) = bg11::unpack_ammo_array(rifle, &q.ammo, &q.powerups);
    println!("back: ammo={a} clips={c} stats={:?}", q.stats);
    assert_eq!((a, c), (30, 6));
}

#[test]
fn ammo_in_foreign_slot() {
    // live 1.1-protocol server: rifle (10) word sent in ammo[1]
    let mut p = PlayerState11::default();
    let rifle = Weapon11Id::Rifle.num() as i32;
    let blaster = Weapon11Id::Blaster.num() as i32;
    p.weapon = rifle;
    p.stats[trem_sim::gen::enums::Stat11::Weapons as usize] = (1 << rifle) | (1 << blaster);
    p.ammo[1] = 24606;
    let s = ps_from_11(&p);
    assert_eq!((s.ammo, s.clips), (30, 6));
    let s2 = ps_from_11(&ps_to_11(&s));
    assert_eq!((s2.ammo, s2.clips), (30, 6));
}
