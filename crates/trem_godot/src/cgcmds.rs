//! cgame `cg_consolecmds.c` (`CG_ConsoleCommand`'s table) and the weapon
//! selection of `cg_weapons.c` (`CG_WeaponSelectable`,
//! `CG_UpgradeSelectable`, `CG_NextWeapon_f`, `CG_PrevWeapon_f`,
//! `CG_Weapon_f`, the selection check of `CG_DrawItemSelect`) with
//! `CG_Respawn`'s reset. cg.weaponSelect is sent as the usercmd weapon
//! (`trap_SetUserCmdValue`); BUTTON_USE_HOLDABLE (+button2) makes PM_Weapon
//! switch to it (< 32) or toggle the upgrade (>= 32).

use trem_sim::gen::constants as k;
use trem_sim::gen::upgrades::UpgradeId;
use trem_sim::kernel::bg;
use trem_sim::kernel::types::PlayerState;

/// cg_consolecmds.c `commands[]` (the model / particle debugging tools
/// testmodel, testgun, nextframe, prevframe, nextskin, prevskin, testPS,
/// destroyTestPS, testTS, destroyTestTS and cgame_memory run on the Godot
/// side: devcmds.rs).
pub const CG_COMMANDS: &[&str] = &[
    "+scores", "-scores", "cgame_memory", "clientlist", "destroyTestPS", "destroyTestTS", "nextframe", "nextskin", "prevframe", "prevskin", "scoresDown", "scoresUp", "sizedown", "sizeup", "testgun", "testmodel", "testPS", "testTS", "ui_menu", "viewpos", "weapnext", "weapon", "weapprev",
];

/// `CG_InitConsoleCommands`' `trap_AddCommand`s for the game's commands
/// ("the game server will interpret these commands, which will be
/// automatically forwarded to the server after they are not recognized
/// locally"): known to the client for tab completion only.
pub const CG_FORWARDED_COMMANDS: &[&str] = &[
    "kill", "ui_messagemode", "ui_messagemode2", "ui_messagemode3", "ui_messagemode4", "say", "say_team", "vsay", "vsay_team", "vsay_local", "m", "mt", "give", "god", "notarget", "noclip", "team", "follow", "levelshot", "setviewpos", "callvote", "vote", "callteamvote", "teamvote", "class", "build", "buy", "sell", "reload", "itemact", "itemdeact", "itemtoggle", "destroy", "deconstruct", "ignore", "unignore",
];

/// Is argv[0] a `CG_ConsoleCommand` command?
pub fn is_cg_command(name: &str) -> bool {
    CG_COMMANDS.iter().any(|c| c.eq_ignore_ascii_case(name))
}

/// cg.weaponSelect / cg.weaponSelectTime.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponSelect {
    /// cg.weaponSelect: a weapon (< 32) or 32 + an upgrade
    pub select: i32,
    /// cg.weaponSelectTime (cg.time)
    pub time: i32,
}

/// `CG_WeaponSelectable`: `BG_InventoryContainsWeapon`.
pub fn weapon_selectable(weapon: i32, ps: &PlayerState) -> bool {
    bg::inventory_contains_weapon(weapon, &ps.stats)
}

/// `CG_UpgradeSelectable`: in the inventory and usable.
pub fn upgrade_selectable(upgrade: i32, ps: &PlayerState) -> bool {
    if !(0..32).contains(&upgrade) || !bg::inventory_contains_upgrade(upgrade, &ps.stats) {
        return false;
    }
    UpgradeId::from_num(upgrade as u8).is_some_and(|u| u.def().usable)
}

fn selectable(sel: i32, ps: &PlayerState) -> bool {
    if sel < 32 {
        weapon_selectable(sel, ps)
    } else {
        upgrade_selectable(sel - 32, ps)
    }
}

impl WeaponSelect {
    /// `CG_NextWeapon_f`: the next selectable weapon or upgrade (0..63
    /// wrapping). Following a player it is the "followprev" client command
    /// instead (returned).
    pub fn next_weapon(&mut self, ps: &PlayerState, time: i32) -> Option<&'static str> {
        if ps.pm_flags & k::PMF_FOLLOW != 0 {
            return Some("followprev");
        }
        self.cycle(ps, time, 1);
        None
    }

    /// `CG_PrevWeapon_f` ("follownext" when following).
    pub fn prev_weapon(&mut self, ps: &PlayerState, time: i32) -> Option<&'static str> {
        if ps.pm_flags & k::PMF_FOLLOW != 0 {
            return Some("follownext");
        }
        self.cycle(ps, time, -1);
        None
    }

    fn cycle(&mut self, ps: &PlayerState, time: i32, dir: i32) {
        self.time = time;
        let original = self.select;
        for _ in 0..64 {
            self.select = (self.select + dir).rem_euclid(64);
            if selectable(self.select, ps) {
                return;
            }
        }
        self.select = original;
    }

    /// `CG_Weapon_f`: `weapon <n>` selects weapon n (1..31) if carried.
    pub fn weapon_f(&mut self, ps: &PlayerState, time: i32, arg: &str) {
        if ps.pm_flags & k::PMF_FOLLOW != 0 {
            return;
        }
        let num = trem_sim::kernel::spawn::c_atoi(arg);
        if !(1..=31).contains(&num) {
            return;
        }
        self.time = time;
        if !bg::inventory_contains_weapon(num, &ps.stats) {
            return; // don't have the weapon
        }
        self.select = num;
    }

    /// `CG_DrawItemSelect`'s first part: make sure that whatever is selected
    /// is actually selectable (alive, not following).
    pub fn validate(&mut self, ps: &PlayerState, time: i32) {
        if ps.stats[trem_sim::gen::enums::Stat::Health as usize] <= 0 || ps.pm_flags & k::PMF_FOLLOW != 0 {
            return;
        }
        if !selectable(self.select, ps) {
            self.next_weapon(ps, time);
        }
    }

    /// `CG_Respawn`: display the weapons and select the one the server says
    /// we are using.
    pub fn respawn(&mut self, ps: &PlayerState, time: i32) {
        self.time = time;
        self.select = ps.weapon;
    }
}

/// `CG_SizeUp_f` / `CG_SizeDown_f`: cg_viewsize in steps of 10 (30..100).
pub fn size_step(viewsize: i32, up: bool) -> i32 {
    if up {
        (viewsize + 10).min(100)
    } else {
        (viewsize - 10).max(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trem_sim::gen::enums::{Stat, Team};
    use trem_sim::gen::weapons::WeaponId;

    /// A human with a rifle, a medkit (usable), a jetpack (usable) and
    /// light armour (not usable).
    fn human() -> PlayerState {
        let mut ps = PlayerState::default();
        ps.stats[Stat::Team as usize] = Team::Humans as i32;
        ps.stats[Stat::Health as usize] = 100;
        ps.stats[Stat::Weapon as usize] = WeaponId::Rifle as i32;
        for u in [UpgradeId::Medkit, UpgradeId::Jetpack, UpgradeId::Larmour] {
            bg::add_upgrade(u as i32, &mut ps.stats);
        }
        ps.weapon = WeaponId::Rifle as i32;
        ps
    }

    #[test]
    fn weapon_cycling_over_an_inventory() {
        let ps = human();
        let blaster = WeaponId::Blaster as i32;
        let rifle = WeaponId::Rifle as i32;
        let mut s = WeaponSelect::default();
        s.respawn(&ps, 5);
        assert_eq!(s, WeaponSelect { select: rifle, time: 5 });
        // next: rifle -> medkit -> jetpack -> blaster (wraps) -> rifle
        let mut seen = Vec::new();
        for t in 0..4 {
            assert_eq!(s.next_weapon(&ps, 10 + t), None);
            seen.push(s.select);
        }
        let medkit = 32 + UpgradeId::Medkit as i32;
        let jetpack = 32 + UpgradeId::Jetpack as i32;
        // blaster (9) < rifle (10): rifle -> medkit -> jetpack -> blaster -> rifle
        let expect = vec![medkit, jetpack, blaster, rifle];
        assert_eq!(seen, expect);
        assert_eq!(s.time, 13);
        // prev goes back the same way
        s.prev_weapon(&ps, 20);
        assert_eq!(s.select, *expect.get(2).unwrap());
        // light armour is never selected (not usable)
        assert!(!upgrade_selectable(UpgradeId::Larmour as i32, &ps));
        // weapon <n>: only carried weapons
        s.weapon_f(&ps, 30, &WeaponId::Shotgun.num().to_string());
        assert_eq!(s.select, *expect.get(2).unwrap());
        s.weapon_f(&ps, 31, &blaster.to_string());
        assert_eq!(s.select, blaster);
        // selling the selected item: CG_DrawItemSelect moves on
        let mut ps2 = ps;
        s.select = medkit;
        bg::remove_upgrade(UpgradeId::Medkit as i32, &mut ps2.stats);
        s.validate(&ps2, 40);
        assert_eq!(s.select, jetpack);
        // nothing selectable: the selection stays
        let mut none = PlayerState::default();
        none.stats[Stat::Health as usize] = 100;
        none.stats[Stat::Weapon as usize] = -1;
        let mut s2 = WeaponSelect { select: 3, time: 0 };
        s2.next_weapon(&none, 1);
        assert_eq!(s2.select, 3);
    }

    #[test]
    fn following_cycles_players() {
        let mut ps = human();
        ps.pm_flags |= k::PMF_FOLLOW;
        let mut s = WeaponSelect::default();
        assert_eq!(s.next_weapon(&ps, 0), Some("followprev"));
        assert_eq!(s.prev_weapon(&ps, 0), Some("follownext"));
        assert_eq!(s.select, 0);
        assert!(is_cg_command("WEAPNEXT"));
        assert_eq!(size_step(100, true), 100);
        assert_eq!(size_step(35, false), 30);
    }
}
