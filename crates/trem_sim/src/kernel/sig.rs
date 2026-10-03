//! Signatures named by `fn:` columns in the sheets (fn tables, MDD §5.7 row 3,
//! §9.3 rank 1: dense index + array of non-capturing fn pointers, no `dyn`).
use super::world::World;

/// 03-impl.tsv `rust_item` cells are only proven to exist (`use path as _`).
pub type Item = ();
/// systems.tsv `run`: one stage of G_RunFrame.
pub type System = fn(&mut World);
/// weapon_behaviors.tsv `fire1..3`: FireWeapon/2/3 for one weapon (ent = shooter).
pub type WeaponFire = fn(&mut World, usize);
/// buildable_behaviors.tsv `think`: called when level.time >= nextthink.
pub type BuildableThink = fn(&mut World, usize);
/// buildable_behaviors.tsv `die`: (world, self, inflictor, attacker, damage, mod).
pub type BuildableDie = fn(&mut World, usize, usize, usize, i32, i32);
/// gentity_t `think` for any entity (same shape as BuildableThink).
pub type Think = fn(&mut World, usize);
/// gentity_t `touch`: (world, self, other).
pub type Touch = fn(&mut World, usize, usize);
/// gentity_t `use`: (world, self, other, activator); C passes NULL as None.
pub type Use = fn(&mut World, usize, Option<usize>, Option<usize>);
/// gentity_t `reached`: a mover arrived at the end of its trajectory.
pub type Reached = fn(&mut World, usize);
/// gentity_t `blocked`: (world, mover, obstacle).
pub type Blocked = fn(&mut World, usize, usize);
/// entity_spawns.tsv `spawn`: SP_* for one classname (spawn vars in World::spawn_vars).
pub type EntitySpawn = fn(&mut World, usize);
/// gentity_t `pain`: (world, self, attacker, damage taken).
pub type Pain = fn(&mut World, usize, usize, i32);
/// client_commands.tsv `handler`: (world, client, argv) with argv[0] the command name.
pub type ClientCmd = fn(&mut World, usize, &[&str]) -> Result<(), String>;
/// admin_commands.tsv `handler`: (world, admin client or None for the
/// console, argv) -> success, as g_admin_cmd_t.handler.
pub type AdminCmd = fn(&mut World, Option<usize>, &[&str]) -> bool;
