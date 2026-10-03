//! Kernel `world` — gentity_t / gclient_t / level_locals_t storage, the
//! SV_Trace entity clip (server/sv_world.c), and the frame loop, which runs the
//! systems sheet's fn table in its topological order (MDD §9.4).

use super::bg::Trajectory;
use super::cm::{self, ClipMap, Trace};
use super::qmath::*;
use super::rng::Rng;
use super::types::*;
use crate::gen::buildables::BuildableId;
use crate::gen::classes::ClassId;
use crate::gen::enums::{EntityKind, Event, SpectatorState, Team};
use crate::gen::maps::MapId;
use crate::gen::missiles::MissileId;
use crate::gen::weapons::WeaponId;
use std::collections::VecDeque;

pub const MAX_CLIENTS: usize = 64;
pub const MAX_GENTITIES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EType {
    #[default]
    General,
    Player,
    Buildable,
    Missile,
    Corpse,
    Location,
    Mover,
    Portal,
    Speaker,
    PushTrigger,
    TeleportTrigger,
    Invisible,
    ParticleSystem,
    AnimMapObj,
    ModelDoor,
    LightFlare,
}

/// `moverState_t` (g_local.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum MoverState {
    #[default]
    Pos1,
    Pos2,
    OneTo2,
    TwoTo1,
    RotatorPos1,
    RotatorPos2,
    Rotator1To2,
    Rotator2To1,
    ModelPos1,
    ModelPos2,
    Model1To2,
    Model2To1,
}

/// One `gentity_t` (the subset Tremulous gameplay touches).
#[derive(Clone, Debug, Default)]
pub struct Entity {
    pub inuse: bool,
    pub etype: EType,
    pub classname: &'static str,
    pub client: bool,
    // entityState / entityShared
    pub origin: Vec3,
    pub origin2: Vec3,
    pub angles: Vec3,
    pub angles2: Vec3,
    pub pos: Trajectory,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub contents: i32,
    pub clipmask: i32,
    pub linked: bool,
    /// svEntity areanum/areanum2 (SV_LinkEntity), computed for brush models:
    /// a door straddling two areas is the portal between them.
    pub areanum: i32,
    pub areanum2: i32,
    /// G_EvaluateAcceleration state (clients): trapper target leading.
    pub evaluate_acceleration: bool,
    pub acceleration: Vec3,
    pub jerk: Vec3,
    pub old_velocity: Vec3,
    pub old_accel: Vec3,
    /// pm.waterlevel / pm.watertype after the last Pmove.
    pub waterlevel: i32,
    pub watertype: i32,
    pub owner: i32,
    pub ground_entity: i32,
    pub weapon: i32,
    pub generic1: i32,
    pub e_flags: i32,
    pub torso_anim: i32,
    pub legs_anim: i32,
    // gameplay
    pub health: i32,
    pub takedamage: bool,
    pub flags: i32,
    pub nextthink: i32,
    pub think: Option<super::sig::BuildableThink>,
    pub die: Option<super::sig::BuildableDie>,
    pub buildable: Option<BuildableId>,
    pub team: Team,
    pub spawned: bool,
    pub powered: bool,
    pub active: bool,
    pub build_time: i32,
    pub parent_node: Option<usize>,
    pub dcc: i32,
    pub time1000: i32,
    pub client_spawn_time: i32,
    pub count: i32,
    pub timestamp: i32,
    pub enemy: Option<usize>,
    pub killed_by: i32,
    pub credits: Vec<i32>,
    pub last_damage_time: i32,
    pub next_regen_time: i32,
    pub last_health: i32,
    pub damage: i32,
    pub splash_damage: i32,
    pub splash_radius: i32,
    pub method_of_death: i32,
    pub splash_mod: i32,
    pub missile: Option<MissileId>,
    pub point_against_world: bool,
    pub turret_aim: Vec3,
    pub turret_spinup: i32,
    pub turret_state: i32,
    pub speed: f32,
    pub overmind_timers: [i32; 3],
    pub free_after_event: bool,
    /// `animTime`: last level.time the buildable toggle bit flipped.
    pub anim_time: i32,
    pub event_time: i32,

    // ---- map entities (g_spawn.c fields, movers, triggers, targets)
    /// `s.origin` / `s.angles` as parsed from the entity lump (movers keep
    /// them while `origin`/`angles` = r.currentOrigin/r.currentAngles move).
    pub s_origin: Vec3,
    pub s_angles: Vec3,
    /// Repeater build-point zone (`usesBuildPointZone` / `buildPointZone`).
    pub uses_bp_zone: bool,
    pub bp_zone: usize,
    /// Marked for deconstruction (g_markDeconstruct) and when.
    pub deconstruct: bool,
    pub deconstruct_time: i32,
    /// `r.currentAngles` (also s.angles for most map entities).
    pub apos: Trajectory,
    /// Inline brush model ("*N") or md3 path from the entity lump.
    pub model: Option<String>,
    pub model2: Option<String>,
    /// `r.bmodel`: clipped against inline model `model_index`.
    pub bmodel: bool,
    /// `s.modelindex` (inline model number for bmodels, else a config index).
    pub model_index: i32,
    pub model_index2: i32,
    pub mover_state: MoverState,
    pub pos1: Vec3,
    pub pos2: Vec3,
    pub movedir: Vec3,
    pub wait: f32,
    pub random: f32,
    pub last_speed: f32,
    pub rotator_angle: f32,
    pub spawnflags: i32,
    pub target: Option<String>,
    pub targetname: Option<String>,
    /// The "team" key: mover team name (not the game Team).
    pub team_name: Option<String>,
    pub message: Option<String>,
    pub target_shader_name: Option<String>,
    pub target_shader_new_name: Option<String>,
    pub teammaster: Option<usize>,
    pub teamchain: Option<usize>,
    pub activator: Option<usize>,
    pub parent: Option<usize>,
    pub next_train: Option<usize>,
    pub clip_brush: Option<usize>,
    pub sound1to2: i32,
    pub sound2to1: i32,
    pub sound_pos1: i32,
    pub sound_pos2: i32,
    pub sound_loop: i32,
    pub noise_index: i32,
    /// `s.loopSound`
    pub loop_sound: i32,
    pub constant_light: i32,
    pub event_parm: i32,
    /// `s.misc`, `s.frame`, `s.time`, `s.clientNum` (entity-type specific).
    pub misc: i32,
    pub frame: i32,
    pub s_time: i32,
    pub s_client_num: i32,
    pub animation: [f32; 4],
    pub stage_team: i32,
    pub stage_stage: i32,
    pub trigger_gravity: i32,
    pub b_triggers: Vec<i32>,
    pub c_triggers: Vec<i32>,
    pub w_triggers: Vec<i32>,
    pub u_triggers: Vec<i32>,
    pub svflags: i32,
    pub last_move_time: i32,
    /// `r.ownerNum` for portal surfaces.
    pub owner_num: i32,
    pub touch: Option<super::sig::Touch>,
    pub pain: Option<super::sig::Pain>,
    /// `target_ent` (hive and its swarm).
    pub target_ent: Option<usize>,
    /// Barricade `shrunkTime`.
    pub shrunk_time: i32,
    /// `physicsBounce` / `nextPhysicsTime` (g_physics.c).
    pub physics_bounce: f32,
    pub next_physics_time: i32,
    pub use_: Option<super::sig::Use>,
    pub reached: Option<super::sig::Reached>,
    pub blocked: Option<super::sig::Blocked>,
    /// `builtBy`: namelog index of the player who built this structure.
    pub built_by: Option<usize>,
    /// `suicideTime` of a build-log revert placeholder (thinks left before
    /// it kills whatever blocks it).
    pub suicide_time: i32,
}

impl Entity {
    /// `r.absmin` as SV_LinkEntity sets it: rotated brush models use their
    /// radius, everything gets the one-unit pad.
    pub fn absmin(&self) -> Vec3 {
        self.abs_bounds().0
    }
    pub fn absmax(&self) -> Vec3 {
        self.abs_bounds().1
    }
    pub fn abs_bounds(&self) -> (Vec3, Vec3) {
        let ang = self.angles;
        let (mut lo, mut hi);
        if self.bmodel && (ang[0] != 0.0 || ang[1] != 0.0 || ang[2] != 0.0) {
            let max = radius_from_bounds(&self.mins, &self.maxs);
            lo = [self.origin[0] - max, self.origin[1] - max, self.origin[2] - max];
            hi = [self.origin[0] + max, self.origin[1] + max, self.origin[2] + max];
        } else {
            lo = add(&self.origin, &self.mins);
            hi = add(&self.origin, &self.maxs);
        }
        for i in 0..3 {
            lo[i] -= 1.0;
            hi[i] += 1.0;
        }
        (lo, hi)
    }
}

/// `RadiusFromBounds`.
pub fn radius_from_bounds(mins: &Vec3, maxs: &Vec3) -> f32 {
    let mut corner = [0.0f32; 3];
    for i in 0..3 {
        let a = mins[i].abs();
        let b = maxs[i].abs();
        corner[i] = if a > b { a } else { b };
    }
    length(&corner)
}

/// One `gclient_t` (pers + sess + per-life state).
#[derive(Clone, Debug, Default)]
pub struct Client {
    pub connected: bool,
    pub name: String,
    pub is_bot: bool,
    pub ps: PlayerState,
    pub pmext: PmoveExt,
    pub cmd: UserCmd,
    pub team: Team,
    pub class_selection: Option<ClassId>,
    pub human_item: Option<WeaponId>,
    pub credit: i32,
    pub score: i32,
    pub kills: i32,
    pub deaths: i32,
    pub spectator: i32,
    pub time100: i32,
    pub time1000: i32,
    pub time10000: i32,
    pub buttons: i32,
    pub oldbuttons: i32,
    pub respawn_time: i32,
    pub last_creep_slow_time: i32,
    pub grab_expiry_time: i32,
    pub last_lock_time: i32,
    pub last_slow_time: i32,
    pub boosted_time: i32,
    pub last_poison_time: i32,
    pub last_poison_client: i32,
    pub poison_immunity_time: i32,
    pub last_poison_clouded_time: i32,
    pub last_medkit_time: i32,
    pub medkit_health_to_restore: i32,
    pub medkit_increment_time: i32,
    pub last_death_location: Vec3,
    pub evolve_health_fraction: f32,
    pub trample_hit: [i32; 20],
    pub trample_pos: usize,
    pub last_crush_time: i32,
    /// damage taken since the presentation layer last looked (screen flash).
    pub damage_flash: i32,
    /// gclient_t damage_from / damage_fromWorld (P_DamageFeedback's
    /// ps.damagePitch / damageYaw).
    pub damage_from: Vec3,
    pub damage_from_world: bool,
    pub last_attacker: i32,
    pub message: Option<(i32, String)>,
    pub noclip: bool,
    pub last_cmd_time: i32,
    pub latched_buttons: i32,
    pub has_healed: bool,
    pub alive_seconds: i32,
    pub suicide_time: i32,
    pub pain_debounce_time: i32,
    /// G_TriggerMenu(MN_A_INFEST / armoury): the presentation layer opens a menu.
    pub wants_menu: bool,
    /// `sess.spectatorClient` (-1: nobody).
    pub spectator_client: i32,
    /// `pers.stickySpec` (cg_stickySpec, default on).
    pub sticky_spec: bool,
    /// `readyToExit` during intermission.
    pub ready_to_exit: bool,
    /// `pers.vote` / `pers.voted` team bitmasks.
    pub vote: i32,
    pub voted: i32,
    /// `airOutTime` (P_WorldEffects).
    pub air_out_time: i32,
    /// `pers.ignoreList` (cg-side filtering moved into G_SayTo).
    pub ignored: Vec<usize>,
    /// `pers.teamChangeTime`, `pers.enterTime`.
    pub team_change_time: i32,
    pub enter_time: i32,
    /// `pers.guid`: a stable pseudo-GUID (dec026; there are no cl_guids).
    pub guid: String,
    /// `pers.localClient`: the host's own player.
    pub local_client: bool,
    /// `pers.admin` (index into `World::admin.admins`).
    pub admin: Option<usize>,
    /// `pers.namelog` (index into `World::namelogs`).
    pub namelog: Option<usize>,
    /// `pers.floodDemerits` / `pers.floodTime` (G_FloodLimited).
    pub flood_demerits: i32,
    pub flood_time: i32,
    /// `inactivityTime` / `inactivityWarning` (ClientInactivityTimer).
    pub inactivity_time: i32,
    pub inactivity_warning: bool,
    /// `sess.restartTeam` (!restart keepteams / switchteams) and
    /// `sess.spectatorTime` (g_session.c).
    pub restart_team: Team,
    pub spectator_time: i32,
    /// `!g_synchronousClients` for a networked human: usercmds arrive from
    /// the server's netchan and run at once (`ClientThink`, client.rs
    /// `client_think`); false: `G_RunClient` runs `cmd` every frame (bots and
    /// the single-process player). dec033.
    pub async_cmds: bool,
    /// `pers.ip` (G_AddressParse of the userinfo "ip"; empty for bots and
    /// the single-process player).
    pub ip: super::admin::Addr,
    /// `pers.useUnlagged` (userinfo cg_unlagged).
    pub use_unlagged: bool,
    /// `pers.teamInfo` (userinfo teamoverlay): the level time of the last
    /// tinfo sent, 0 when off.
    pub team_info: i32,
    /// `unlaggedHist` / `unlaggedBackup` / `unlaggedCalc` / `unlaggedTime` / `oldOrigin`.
    pub unlagged: super::unlagged::ClientUnlagged,
}

/// What `ClientConnect` reads from the userinfo (dec033).
#[derive(Clone, Debug, Default)]
pub struct ConnectInfo {
    pub name: String,
    /// cl_guid (32 hex digits for a networked client)
    pub guid: String,
    /// the userinfo "ip": "localhost" for the listen server's own client
    pub ip: String,
    /// g_password / userinfo password
    pub password: String,
    pub use_unlagged: bool,
    pub team_info: bool,
}

/// `level_locals_t` plus the g_* cvars the rules read.
#[derive(Clone, Debug, Default)]
pub struct Level {
    pub time: i32,
    /// g_doWarmup / g_warmup (seconds), `level.warmupTime` (restart match
    /// at this time) and the CS_WARMUP configstring (-1: no warmup)
    pub do_warmup: bool,
    pub warmup: i32,
    pub warmup_time: i32,
    pub cs_warmup: i32,
    pub previous_time: i32,
    pub framenum: i32,
    pub start_time: i32,
    pub msec: i32,
    pub num_alien_spawns: i32,
    pub num_human_spawns: i32,
    pub num_live_alien_clients: i32,
    pub num_live_human_clients: i32,
    pub num_alien_clients: i32,
    pub num_human_clients: i32,
    pub alien_bp_queue: i32,
    pub human_bp_queue: i32,
    /// `level.alienBuildPoints` / `level.humanBuildPoints`: what is left after
    /// the queue and every standing structure (G_CalculateBuildPoints).
    pub alien_bp_left: i32,
    pub human_bp_left: i32,
    /// `level.buildPointZones` (repeater zones).
    pub bp_zones: Vec<BpZone>,
    /// `level.markedBuildables` / `numBuildablesForRemoval` from the last G_CanBuild.
    pub marked_buildables: Vec<usize>,
    pub num_for_removal: usize,
    /// g_markDeconstruct (3: mark, replace anything), g_humanRepeater* cvars.
    pub mark_deconstruct: i32,
    pub repeater_bp: i32,
    pub repeater_queue_time: i32,
    pub repeater_max_zones: i32,
    /// g_suddenDeathTime (minutes) and the sudden death state.
    pub sudden_death_time: i32,
    pub sudden_death_begin_time: i32,
    pub sudden_death_warning: i32,
    pub timelimit_warning: i32,
    pub cheats: bool,
    pub alien_next_queue_time: i32,
    pub human_next_queue_time: i32,
    pub alien_credits: i32,
    pub human_credits: i32,
    pub alien_stage: i32,
    pub human_stage: i32,
    pub avg_alien_clients: f32,
    pub avg_human_clients: f32,
    pub num_alien_samples: i32,
    pub num_human_samples: i32,
    pub alien_spawn_queue: VecDeque<usize>,
    pub human_spawn_queue: VecDeque<usize>,
    pub winner: Option<Team>,
    pub intermission_queued: i32,
    pub overmind_muted: bool,
    pub intermission_time: i32,
    /// FindIntermissionPoint results.
    pub intermission_origin: Vec3,
    pub intermission_angle: Vec3,
    /// CheckIntermissionExit: someone readied and when.
    pub ready_to_exit: bool,
    pub exit_time: i32,
    /// votes, indexed by team as `level.vote*[ team ]`.
    pub votes: [super::vote::VoteState; 3],
    /// `level.surrenderTeam` (admitdefeat).
    pub surrender_team: Option<Team>,
    /// `level.humanBaseAttackTimer` (EV_DCC_ATTACK throttle).
    pub human_base_attack_timer: i32,
    /// map change requested by a passed vote ("map X"); None: restart.
    pub next_map: Option<String>,
    /// g_allowVote, g_suddenDeathVotePercent, g_suddenDeathVoteDelay.
    pub allow_vote: bool,
    pub sudden_death_vote_percent: i32,
    pub sudden_death_vote_delay: i32,
    /// map names the host can load (G_MapExists).
    pub map_list: Vec<String>,
    /// g_specChat.
    pub spec_chat: bool,
    /// g_teamForceBalance (default off).
    pub team_force_balance: bool,
    /// ExitLevel happened: the host reloads the map (map_restart; there is no
    /// rotation configured by default).
    pub exited: bool,
    pub alien_next_stage: i32,
    pub human_next_stage: i32,
    /// stage-up announcements for the presentation layer (team, new stage 1-based).
    pub stage_events: Vec<(Team, i32)>,
    /// player_die's function-static death animation counter.
    pub death_cycle: i32,
    pub timelimit: i32,
    pub freekill_period: i32,
    /// target_alien_win / target_human_win.
    pub uncond_alien_win: bool,
    pub uncond_human_win: bool,
    /// target_location chain head (linked through next_train).
    pub location_head: Option<usize>,
    /// AddRemap: (old shader, new shader, time offset) from G_UseTargets.
    pub shader_remaps: Vec<(String, String, f32)>,
    /// The team of the last win (for trigger_win), set by LogExit.
    pub last_win: Option<Team>,
    // cvars
    pub g_speed: f32,
    pub g_gravity: f32,
    pub g_knockback: f32,
    pub alien_bp: i32,
    pub human_bp: i32,
    pub alien_stage2_thresh: i32,
    pub alien_stage3_thresh: i32,
    pub human_stage2_thresh: i32,
    pub human_stage3_thresh: i32,
    pub alien_max_stage: i32,
    pub human_max_stage: i32,
    pub friendly_fire: bool,
    pub alien_queue_time: i32,
    pub human_queue_time: i32,
    /// `level.layout` (G_LayoutSelect): "" or "*BUILTIN*" keep the map's own structures.
    pub layout: String,
    /// `level.pausedTime` (admin pause) and G_RunFrame's static ptime3000.
    pub paused_time: i32,
    pub ptime3000: i32,
    /// `level.alienTeamLocked` / `level.humanTeamLocked` (admin lock).
    pub alien_team_locked: bool,
    pub human_team_locked: bool,
    /// `level.buildLog` ring (MAX_BUILDLOG), `level.buildId`, `level.numBuildLogs`.
    pub build_log: Vec<super::layout::BuildLog>,
    pub build_id: i32,
    pub num_build_logs: i32,
    // server/admin cvars (g_main.c gameCvarTable defaults)
    pub g_admin: String,
    pub admin_temp_ban: String,
    pub admin_max_ban: String,
    pub inactivity: i32,
    pub flood_min_time: i32,
    pub flood_max_demerits: i32,
    pub min_name_change_period: i32,
    pub max_name_changes: i32,
    pub vote_limit: i32,
    pub layouts: String,
    pub layout_auto: bool,
    pub private_messages: bool,
    pub public_admin_messages: bool,
    pub censorship: String,
    pub emoticons_allowed_in_names: bool,
    pub lock_teams_at_start: bool,
    /// g_logFile / g_logFileSync / dedicated / g_tag, `level.logFile` (open)
    /// and the lines not yet written (log.rs).
    pub log_file: String,
    pub log_sync: bool,
    pub log_open: bool,
    pub log_buffer: String,
    pub dedicated: bool,
    pub tag: String,
    /// g_password (ClientConnect; empty or "none": no password).
    pub password: String,
    /// g_unlagged and `level.unlaggedIndex` / `level.unlaggedTimes` (unlagged.rs).
    pub unlagged: bool,
    pub unlagged_index: usize,
    pub unlagged_times: [i32; super::unlagged::MAX_UNLAGGED_MARKERS],
    /// g_admin.c's `adminLog` line being built (admin_log_start .. end).
    pub admin_log: String,
    /// `level.alienStage2Time` .. `level.humanStage3Time` (G_SendGameStat).
    pub alien_stage2_time: i32,
    pub alien_stage3_time: i32,
    pub human_stage2_time: i32,
    pub human_stage3_time: i32,
}

/// `trap_FS_*` for the game module: the host implements it over its pk3
/// search path and a user-writable folder (dec026). Paths are game paths
/// ("admin.dat", "layouts/atcs/mine.dat").
pub trait Fs {
    /// `trap_FS_FOpenFile(FS_READ)` + `trap_FS_Read`.
    fn read(&self, path: &str) -> Option<String>;
    /// `trap_FS_FOpenFile(FS_WRITE)` + `trap_FS_Write`; false if it cannot be opened.
    fn write(&mut self, path: &str, text: &str) -> bool;
    /// trap_FS_FOpenFile(FS_APPEND) + trap_FS_Write: add `text` to the
    /// end of the file (created if missing); false if it cannot be opened.
    fn append(&mut self, path: &str, text: &str) -> bool {
        let old = self.read(path).unwrap_or_default();
        self.write(path, &(old + text))
    }
    /// `trap_FS_GetFileList(dir, ext)`: file names (with extension) in `dir`.
    fn list(&self, dir: &str, ext: &str) -> Vec<String>;
    /// `trap_FS_FOpenFile(path, NULL, FS_READ) > 0`.
    fn exists(&self, path: &str) -> bool {
        self.read(path).is_some_and(|t| !t.is_empty())
    }
}

/// `zap_t`: an advanced marauder's zap chain (drawn as beams).
#[derive(Clone, Debug, Default)]
pub struct Zap {
    pub creator: usize,
    pub targets: Vec<usize>,
    pub distances: Vec<f32>,
    pub time_to_live: i32,
}

/// `buildPointZone_t`.
#[derive(Clone, Copy, Debug, Default)]
pub struct BpZone {
    pub active: bool,
    pub total: i32,
    pub queued: i32,
    pub next_queue_time: i32,
}

/// A temp entity / G_AddEvent, kept for the presentation layer (sounds, fx).
#[derive(Clone, Copy, Debug)]
pub struct GameEvent {
    pub time: i32,
    pub event: Event,
    pub origin: Vec3,
    pub origin2: Vec3,
    pub entity: i32,
    pub other: i32,
    pub parm: i32,
    pub weapon: i32,
    /// es->generic1 (weapon mode) and es->torsoAnim (lcannon charge, 0-255)
    /// of the entity the event came from.
    pub generic1: i32,
    pub charge: i32,
    /// A playerstate event of the entity's client (`EF_PLAYER_EVENT`): its
    /// owner predicts it, so a server leaves it out of the owner's
    /// snapshots (`SendPendingPredictableEvents`' SVF_NOTSINGLECLIENT).
    pub predictable: bool,
}

/// A map placement: structure, player start or intermission point.
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    pub kind: EntityKind,
    pub team: Team,
    pub buildable: Option<BuildableId>,
    pub origin: Vec3,
    pub yaw: f32,
}

pub struct World {
    /// Sheet map (for BSP worlds: the first sheet map, used only for defaults).
    pub map: MapId,
    pub map_name: String,
    /// Whether the waypoints sheet covers this map (bots steer directly otherwise).
    pub has_waypoints: bool,
    pub spots: Vec<Spot>,
    pub cm: ClipMap,
    pub ents: Vec<Entity>,
    pub clients: Vec<Client>,
    pub num_entities: usize,
    pub level: Level,
    pub rng: Rng,
    pub events: Vec<GameEvent>,
    pub bots: super::bot::Bots,
    /// Bot navigation graph (built on first use).
    pub nav: Option<super::nav::Nav>,
    /// Event cursor for presentation consumers.
    pub event_base: u64,
    /// Configstring tables: CS_SOUNDS, CS_MODELS, CS_PARTICLE_SYSTEMS,
    /// CS_SHADERS, CS_LOCATIONS (index 0 is unused, as in the engine).
    pub sound_names: Vec<String>,
    pub model_names: Vec<String>,
    pub particle_names: Vec<String>,
    pub shader_names: Vec<String>,
    pub location_names: Vec<String>,
    /// Live zap chains (g_weapon.c zaps[]).
    pub zaps: Vec<Zap>,
    /// `level.spawnVars` while an entity is being spawned.
    pub spawn_vars: Vec<(String, String)>,
    /// "message" key of worldspawn and the misc_portal etc. text for HUDs.
    pub center_prints: Vec<(i32, Option<Team>, Option<usize>, String)>,
    /// "print" server commands (console/feed lines): (time, team, client, text).
    pub prints: Vec<(i32, Option<Team>, Option<usize>, String)>,
    /// "chat" server commands: (time, to, from, mode, line); the host drains them.
    /// `from` is `CONSOLE` for messages from the server console.
    pub chat: Vec<(i32, usize, usize, super::cmds::SayMode, String)>,
    /// g_admin.c state: levels, admins, bans, custom commands (admin.dat).
    pub admin: super::admin::AdminState,
    /// `level.namelogs` (g_namelog.c).
    pub namelogs: Vec<super::admin::NameLog>,
    /// G_Printf / G_admin_print(NULL) output for the server console; the
    /// console caller drains it.
    pub console: String,
    /// `trap_SendConsoleCommand( EXEC_APPEND, ... )`: lines for the server
    /// console (svcmds::run_exec runs the game's own, the host the rest).
    pub exec: Vec<String>,
    /// `trap_DropClient` requests for the server (networked clients):
    /// (client, reason); the single-process host ignores them.
    pub drops: Vec<(usize, String)>,
    /// other `trap_SendServerCommand` text for networked clients (client,
    /// command), e.g. the ban's `disconnect "<reason>"`; drained by the server.
    pub server_commands: Vec<(Option<usize>, String)>,
    /// The host's file system (None: every read fails, writes are refused).
    pub fs: Option<Box<dyn Fs>>,
    /// `trap_RealTime`: wall-clock seconds since the epoch, set by the host.
    pub real_time: i32,
    /// G_LoadCensors pairs (term, replacement); g_censorship is empty by default.
    pub censors: Vec<(String, String)>,
    /// `level.emoticons` names (BG_LoadEmoticons), for G_IsEmoticon.
    pub emoticons: Vec<String>,
    /// A view World of a protocol 69 (Tremulous 1.1.0) game: its tables and
    /// physics are 1.1's (dec041, kernel::v11); set by the cgame.
    pub vanilla: bool,
}

/// `chat` sender for the server console (`ent == NULL`).
pub const CONSOLE: usize = usize::MAX;

fn parse_cvar(s: &str) -> i32 {
    s.parse().unwrap_or(0)
}

impl World {
    /// A world on a sheet-authored map (brushes + map_entities sheets).
    pub fn new(map: MapId) -> Self {
        let md = map.def();
        let mut spots = Vec::new();
        for me in crate::gen::map_entities::MAP_ENTITIES.iter().filter(|e| e.map == map) {
            spots.push(Spot { kind: me.kind, team: me.team, buildable: me.buildable, origin: me.origin, yaw: me.yaw });
        }
        let mut w = Self::build(map, cm::from_sheet(map), spots, md.gravity, md.alien_bp, md.human_bp, true);
        w.map_name = md.id.to_string();
        w.has_waypoints = crate::gen::waypoints::WAYPOINTS.iter().any(|x| x.map == map);
        w
    }

    /// A world on an IBSP map: collision from the BSP, structures and
    /// intermission points from its entity lump (G_SpawnEntitiesFromString
    /// for the classnames the game layer knows), worldspawn overrides for
    /// gravity and build points.
    pub fn from_bsp(bsp: &super::bsp::Bsp, name: &str) -> Self {
        Self::from_bsp_with(bsp, name, None, None)
    }

    /// `G_InitGame` on an IBSP map with the host's file system and a layout
    /// choice: `g_layouts` Some(list) runs G_LayoutSelect (with g_layoutAuto,
    /// an empty list picks among all of the map's layouts) before the
    /// entities spawn, then G_LayoutLoad. None keeps the map's structures.
    pub fn from_bsp_with(bsp: &super::bsp::Bsp, name: &str, fs: Option<Box<dyn Fs>>, g_layouts: Option<(&str, bool)>) -> Self {
        let ents = bsp.entity_list();
        let (spots, gravity, abp, hbp) = Self::bsp_spots(&ents);
        let mut w = Self::build(MapId::ALL[0], bsp.clip_map(), spots, gravity, abp, hbp, false);
        w.map_name = name.to_string();
        w.has_waypoints = false;
        w.fs = fs;
        if let Some((list, auto)) = g_layouts {
            w.level.layout_auto = auto;
            super::layout::layout_select(&mut w, list);
        }
        super::spawn::spawn_entities(&mut w, &ents);
        super::layout::layout_load(&mut w);
        w
    }

    /// The entity lump's placements and worldspawn overrides (gravity,
    /// alien / human build points).
    fn bsp_spots(ents: &[std::collections::BTreeMap<String, String>]) -> (Vec<Spot>, f32, i32, i32) {
        use crate::gen::buildables::BUILDABLES;
        let mut spots = Vec::new();
        let (mut gravity, mut abp, mut hbp) = (k::DEFAULT_GRAVITY as f32, parse_cvar(k::DEFAULT_ALIEN_BUILDPOINTS), parse_cvar(k::DEFAULT_HUMAN_BUILDPOINTS));
        for e in ents {
            let class = e.get("classname").map(|s| s.as_str()).unwrap_or("");
            let origin = e.get("origin").and_then(|o| super::bsp::parse_vec(o)).unwrap_or([0.0; 3]);
            let yaw = e.get("angle").and_then(|a| a.parse().ok()).or_else(|| e.get("angles").and_then(|a| super::bsp::parse_vec(a)).map(|v| v[1])).unwrap_or(0.0);
            match class {
                "worldspawn" => {
                    if let Some(g) = e.get("gravity").and_then(|g| g.parse().ok()) {
                        gravity = g;
                    }
                    if let Some(b) = e.get("alienBuildPoints").and_then(|g| g.parse().ok()) {
                        abp = b;
                    }
                    if let Some(b) = e.get("humanBuildPoints").and_then(|g| g.parse().ok()) {
                        hbp = b;
                    }
                }
                "info_player_intermission" | "info_player_start" | "info_player_deathmatch" => {
                    if !spots.iter().any(|s: &Spot| s.kind == EntityKind::PlayerStart) {
                        spots.push(Spot { kind: EntityKind::PlayerStart, team: Team::None, buildable: None, origin, yaw });
                    }
                }
                "info_alien_intermission" => spots.push(Spot { kind: EntityKind::Intermission, team: Team::Aliens, buildable: None, origin, yaw }),
                "info_human_intermission" => spots.push(Spot { kind: EntityKind::Intermission, team: Team::Humans, buildable: None, origin, yaw }),
                _ => {
                    // (the 1.1-only presentation rows, in no stage, are not legacy entities, dec041)
                    for bd in BUILDABLES.iter().filter(|d| d.stages != 0) {
                        let id = BuildableId::from_num(bd.num).unwrap();
                        if id.cold().entity_name == class {
                            spots.push(Spot { kind: EntityKind::Buildable, team: bd.team, buildable: Some(id), origin, yaw });
                        }
                    }
                }
            }
        }
        (spots, gravity, abp, hbp)
    }

    /// A networked client's view of an IBSP map (cgview.rs, dec033): the
    /// collision model, inline models and placements, nothing spawned; the
    /// snapshots and configstrings fill the rest.
    pub fn client_view_from_bsp(bsp: &super::bsp::Bsp, name: &str) -> Self {
        let ents = bsp.entity_list();
        let (spots, gravity, abp, hbp) = Self::bsp_spots(&ents);
        let mut w = Self::build(MapId::ALL[0], bsp.clip_map(), spots, gravity, abp, hbp, false);
        w.map_name = name.to_string();
        w.console.clear();
        w
    }

    /// A networked client's view of a sheet map.
    pub fn client_view(map: MapId) -> Self {
        let md = map.def();
        let mut spots = Vec::new();
        for me in crate::gen::map_entities::MAP_ENTITIES.iter().filter(|e| e.map == map) {
            spots.push(Spot { kind: me.kind, team: me.team, buildable: me.buildable, origin: me.origin, yaw: me.yaw });
        }
        let mut w = Self::build(map, cm::from_sheet(map), spots, md.gravity, md.alien_bp, md.human_bp, false);
        w.map_name = md.id.to_string();
        w.console.clear();
        w
    }

    fn build(map: MapId, cm: ClipMap, spots: Vec<Spot>, gravity: f32, alien_bp: i32, human_bp: i32, spawn_spots: bool) -> Self {
        // a new game: nothing disabled until SP_worldspawn's keys (BG_InitAllowedGameElements)
        super::bg::init_allowed_game_elements("", "", "");
        let mut w = World {
            map,
            map_name: String::new(),
            has_waypoints: false,
            spots,
            cm,
            ents: vec![Entity::default(); MAX_GENTITIES],
            clients: vec![Client::default(); MAX_CLIENTS],
            num_entities: MAX_CLIENTS,
            level: Level::default(),
            rng: Rng::default(),
            events: Vec::new(),
            bots: Default::default(),
            nav: None,
            event_base: 0,
            sound_names: vec![String::new()],
            model_names: vec![String::new()],
            particle_names: vec![String::new()],
            shader_names: vec![String::new()],
            location_names: vec![String::new()],
            spawn_vars: Vec::new(),
            center_prints: Vec::new(),
            chat: Vec::new(),
            prints: Vec::new(),
            zaps: Vec::new(),
            admin: Default::default(),
            namelogs: Vec::new(),
            console: String::new(),
            exec: Vec::new(),
            drops: Vec::new(),
            server_commands: Vec::new(),
            fs: None,
            real_time: 0,
            censors: Vec::new(),
            emoticons: Vec::new(),
            vanilla: false,
        };
        let l = &mut w.level;
        l.g_speed = 320.0;
        l.g_gravity = gravity;
        l.g_knockback = 1000.0;
        l.allow_vote = true;
        l.spec_chat = true;
        l.sudden_death_vote_percent = 74;
        l.sudden_death_vote_delay = 180;
        l.alien_bp = alien_bp;
        l.human_bp = human_bp;
        l.alien_stage2_thresh = parse_cvar(k::DEFAULT_ALIEN_STAGE2_THRESH);
        l.alien_stage3_thresh = parse_cvar(k::DEFAULT_ALIEN_STAGE3_THRESH);
        l.human_stage2_thresh = parse_cvar(k::DEFAULT_HUMAN_STAGE2_THRESH);
        l.human_stage3_thresh = parse_cvar(k::DEFAULT_HUMAN_STAGE3_THRESH);
        l.alien_max_stage = parse_cvar(k::DEFAULT_ALIEN_MAX_STAGE);
        l.human_max_stage = parse_cvar(k::DEFAULT_HUMAN_MAX_STAGE);
        l.alien_queue_time = parse_cvar(k::DEFAULT_ALIEN_QUEUE_TIME);
        l.human_queue_time = parse_cvar(k::DEFAULT_HUMAN_QUEUE_TIME);
        l.freekill_period = parse_cvar(k::DEFAULT_FREEKILL_PERIOD);
        l.mark_deconstruct = 3; // g_markDeconstruct default
        l.repeater_bp = parse_cvar(k::DEFAULT_HUMAN_REPEATER_BUILDPOINTS);
        l.repeater_queue_time = parse_cvar(k::DEFAULT_HUMAN_REPEATER_QUEUE_TIME);
        l.repeater_max_zones = parse_cvar(k::DEFAULT_HUMAN_REPEATER_MAX_ZONES);
        l.bp_zones = vec![BpZone::default(); l.repeater_max_zones.max(0) as usize];
        l.sudden_death_time = 0; // g_suddenDeathTime default
        l.sudden_death_begin_time = l.sudden_death_time * 60000;
        l.timelimit = 0; // g_timelimit: off for the sandbox
        l.time = 0;
        l.start_time = 0;
        l.build_log = vec![Default::default(); super::layout::MAX_BUILDLOG];
        l.g_admin = "admin.dat".into();
        l.admin_temp_ban = "2m".into();
        l.admin_max_ban = "2w".into();
        l.flood_min_time = 2000;
        l.flood_max_demerits = 5000;
        l.min_name_change_period = 5;
        // g_main.c cvarTable: g_warmup 10, g_doWarmup 0
        l.warmup = 10;
        l.cs_warmup = -1;
        l.max_name_changes = 5;
        l.vote_limit = 5;
        l.layout_auto = true;
        l.private_messages = true;
        l.public_admin_messages = true;
        l.emoticons_allowed_in_names = true;
        l.log_file = super::log::DEFAULT_LOG_FILE.into();
        l.unlagged = true; // g_unlagged default
        l.tag = super::log::DEFAULT_TAG.into();
        for i in 0..MAX_GENTITIES {
            w.ents[i].credits = vec![0; MAX_CLIENTS];
            w.ents[i].owner = ENTITYNUM_NONE;
            w.ents[i].ground_entity = ENTITYNUM_NONE;
        }
        // G_admin_readconfig: no file system yet, so the default levels
        super::admin::readconfig(&mut w, None);
        w.console.clear();
        if spawn_spots {
            w.spawn_map_entities();
        }
        w
    }

    /// `trap_Cvar_Set` for the g_* cvars the rules read (server.cfg, map
    /// rotation command sections, `set` in the console). Returns false for
    /// a cvar this port does not model.
    pub fn set_cvar(&mut self, name: &str, value: &str) -> bool {
        let i = super::spawn::c_atoi(value);
        let f = super::spawn::c_atof(value) as f32;
        let l = &mut self.level;
        match name.to_ascii_lowercase().as_str() {
            "timelimit" => l.timelimit = i,
            "g_suddendeathtime" => {
                l.sudden_death_time = i;
                l.sudden_death_begin_time = i * 60000;
            }
            "g_friendlyfire" => l.friendly_fire = i != 0,
            "g_teamforcebalance" => l.team_force_balance = i != 0,
            "g_cheats" | "sv_cheats" => l.cheats = i != 0,
            "g_allowvote" => l.allow_vote = i != 0,
            "g_specchat" => l.spec_chat = i != 0,
            "g_suddendeathvotepercent" => l.sudden_death_vote_percent = i,
            "g_suddendeathvotedelay" => l.sudden_death_vote_delay = i,
            "g_markdeconstruct" => l.mark_deconstruct = i,
            "g_freefundperiod" => l.freekill_period = i,
            "g_alienbuildpoints" => l.alien_bp = i,
            "g_humanbuildpoints" => l.human_bp = i,
            "g_alienstage2threshold" => l.alien_stage2_thresh = i,
            "g_alienstage3threshold" => l.alien_stage3_thresh = i,
            "g_humanstage2threshold" => l.human_stage2_thresh = i,
            "g_humanstage3threshold" => l.human_stage3_thresh = i,
            "g_alienmaxstage" => l.alien_max_stage = i,
            "g_humanmaxstage" => l.human_max_stage = i,
            "g_humanrepeaterbuildpoints" => l.repeater_bp = i,
            "g_speed" => l.g_speed = f,
            "g_gravity" => l.g_gravity = f,
            "g_knockback" => l.g_knockback = f,
            "g_admin" => l.g_admin = value.to_string(),
            "g_admintempban" => l.admin_temp_ban = value.to_string(),
            "g_adminmaxban" => l.admin_max_ban = value.to_string(),
            "g_inactivity" => l.inactivity = i,
            "g_floodmintime" => l.flood_min_time = i,
            "g_floodmaxdemerits" => l.flood_max_demerits = i,
            "g_minnamechangeperiod" => l.min_name_change_period = i,
            "g_dowarmup" => l.do_warmup = i != 0,
            "g_warmup" => l.warmup = i,
            "g_maxnamechanges" => l.max_name_changes = i,
            "g_votelimit" => l.vote_limit = i,
            "g_layouts" => l.layouts = value.to_string(),
            "g_layoutauto" => l.layout_auto = i != 0,
            "g_privatemessages" => l.private_messages = i != 0,
            "g_publicadminmessages" => l.public_admin_messages = i != 0,
            "g_censorship" => l.censorship = value.to_string(),
            "g_emoticonsallowedinnames" => l.emoticons_allowed_in_names = i != 0,
            "g_lockteamsatstart" => l.lock_teams_at_start = i != 0,
            "g_logfile" => l.log_file = value.to_string(),
            "g_logfilesync" => l.log_sync = i != 0,
            "dedicated" => l.dedicated = i != 0,
            "g_tag" => l.tag = value.to_string(),
            "g_password" => l.password = value.to_string(),
            "g_unlagged" => l.unlagged = i != 0,
            _ => return false,
        }
        true
    }

    /// A line of server console commands (`set x y; seta z w; ...`): the
    /// cvar sets are applied, anything else is returned unapplied.
    pub fn exec_server_commands(&mut self, text: &str) -> Vec<String> {
        let mut rest = Vec::new();
        for cmd in text.split([';', '\n']).map(str::trim).filter(|c| !c.is_empty()) {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            let (name, value) = match parts.as_slice() {
                [s, n, v, ..] if matches!(s.to_ascii_lowercase().as_str(), "set" | "seta" | "sets" | "setu") => (*n, v.trim_matches('"')),
                [n, v, ..] => (*n, v.trim_matches('"')),
                _ => {
                    rest.push(cmd.to_string());
                    continue;
                }
            };
            if !self.set_cvar(name, value) {
                rest.push(cmd.to_string());
            }
        }
        rest
    }

    // ------------------------------------------------------- entity lifecycle
    /// `G_Spawn`.
    pub fn spawn(&mut self) -> usize {
        for i in MAX_CLIENTS..MAX_GENTITIES - 2 {
            if !self.ents[i].inuse && (i >= self.num_entities || self.ents[i].event_time + 1000 < self.level.time || self.level.time < 2000) {
                self.init_entity(i);
                if i >= self.num_entities {
                    self.num_entities = i + 1;
                }
                return i;
            }
        }
        panic!("G_Spawn: no free entities");
    }
    fn init_entity(&mut self, i: usize) {
        let credits = std::mem::take(&mut self.ents[i].credits);
        self.ents[i] = Entity { inuse: true, owner: ENTITYNUM_NONE, ground_entity: ENTITYNUM_NONE, killed_by: ENTITYNUM_NONE, credits, ..Default::default() };
        self.ents[i].credits.iter_mut().for_each(|c| *c = 0);
        if self.ents[i].credits.len() != MAX_CLIENTS {
            self.ents[i].credits = vec![0; MAX_CLIENTS];
        }
    }
    /// `G_FreeEntity`.
    pub fn free(&mut self, i: usize) {
        let e = &mut self.ents[i];
        e.inuse = false;
        e.linked = false;
        e.event_time = self.level.time;
        e.think = None;
        e.die = None;
        e.touch = None;
        e.pain = None;
        e.use_ = None;
        e.reached = None;
        e.blocked = None;
        e.classname = "freed";
        // G_FreeEntity memsets the entity. Without clearing the identity
        // fields a freed buildable stays an ET_BUILDABLE with its parentNode,
        // so the power-zone BP scans (G_FindPower, G_CalculateBuildPoints)
        // kept charging its build points to the reactor forever and live
        // structures lost power mid-game.
        e.etype = EType::General;
        e.buildable = None;
        e.parent_node = None;
        e.powered = false;
        e.spawned = false;
        e.health = 0;
        e.uses_bp_zone = false;
    }

    // ------------------------------------------------------- configstrings
    fn config_index(table: &mut Vec<String>, name: &str) -> i32 {
        if name.is_empty() {
            return 0;
        }
        if let Some(i) = table.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            return i as i32;
        }
        table.push(name.to_string());
        table.len() as i32 - 1
    }
    /// `G_SoundIndex`.
    pub fn sound_index(&mut self, name: &str) -> i32 {
        Self::config_index(&mut self.sound_names, name)
    }
    /// `G_ModelIndex`.
    pub fn model_index(&mut self, name: &str) -> i32 {
        Self::config_index(&mut self.model_names, name)
    }
    /// `G_ParticleSystemIndex`.
    pub fn particle_index(&mut self, name: &str) -> i32 {
        Self::config_index(&mut self.particle_names, name)
    }
    /// `G_ShaderIndex`.
    pub fn shader_index(&mut self, name: &str) -> i32 {
        Self::config_index(&mut self.shader_names, name)
    }
    pub fn sound_name(&self, i: i32) -> Option<&str> {
        self.sound_names.get(i as usize).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }
    pub fn add_event(&mut self, ent: usize, event: Event, parm: i32) {
        let e = &self.ents[ent];
        self.events.push(GameEvent { time: self.level.time, event, origin: e.origin, origin2: e.origin2, entity: ent as i32, other: -1, parm, weapon: e.weapon, generic1: e.generic1, charge: e.torso_anim, predictable: false });
    }
    /// A playerstate event of client `ent` (ClientEvents' copy of
    /// `ps.events`, EF_PLAYER_EVENT).
    pub fn add_predictable_event(&mut self, ent: usize, event: Event, parm: i32) {
        self.add_event(ent, event, parm);
        if let Some(e) = self.events.last_mut() {
            e.predictable = true;
        }
    }
    pub fn temp_event(&mut self, origin: Vec3, event: Event, other: i32, parm: i32, weapon: i32) {
        self.events.push(GameEvent { time: self.level.time, event, origin, origin2: [0.0; 3], entity: -1, other, parm, weapon, generic1: 0, charge: 0, predictable: false });
    }
    /// A temp event carrying the source entity's weapon mode and charge.
    pub fn temp_event_from(&mut self, origin: Vec3, event: Event, other: i32, parm: i32, src: usize) {
        let (weapon, generic1, charge) = (self.ents[src].weapon, self.ents[src].generic1, self.ents[src].torso_anim);
        self.events.push(GameEvent { time: self.level.time, event, origin, origin2: [0.0; 3], entity: -1, other, parm, weapon, generic1, charge, predictable: false });
    }
    /// Events newer than `cursor`; returns the new cursor (presentation glue).
    pub fn drain_events(&mut self) -> Vec<GameEvent> {
        self.event_base += self.events.len() as u64;
        std::mem::take(&mut self.events)
    }

    // ------------------------------------------------------------ collision
    /// `trap_EntitiesInBox` over linked entities.
    pub fn entities_in_box(&self, mins: &Vec3, maxs: &Vec3) -> Vec<usize> {
        let mut out = Vec::new();
        for i in 0..self.num_entities {
            let e = &self.ents[i];
            if !e.inuse || !e.linked {
                continue;
            }
            let (lo, hi) = e.abs_bounds();
            if bounds_intersect(&lo, &hi, mins, maxs) {
                out.push(i);
            }
        }
        out
    }

    /// `SV_Trace`: world clip, then SV_ClipMoveToEntities.
    pub fn trace(&self, start: &Vec3, mins: Option<&Vec3>, maxs: Option<&Vec3>, end: &Vec3, pass: i32, mask: i32) -> Trace {
        self.trace_ex(start, mins, maxs, end, pass, mask, &|_| false)
    }

    /// `SV_Trace` treating the entities `unlinked` accepts as unlinked.
    pub fn trace_ex(&self, start: &Vec3, mins: Option<&Vec3>, maxs: Option<&Vec3>, end: &Vec3, pass: i32, mask: i32, unlinked: &dyn Fn(usize) -> bool) -> Trace {
        let zero = [0.0f32; 3];
        let mins = mins.unwrap_or(&zero);
        let maxs = maxs.unwrap_or(&zero);
        let mut tr = self.cm.box_trace(start, end, Some(mins), Some(maxs), mask);
        tr.entity_num = if tr.fraction != 1.0 { ENTITYNUM_WORLD } else { ENTITYNUM_NONE };
        if tr.fraction == 0.0 {
            return tr;
        }
        let mut bmins = [0.0f32; 3];
        let mut bmaxs = [0.0f32; 3];
        for i in 0..3 {
            if end[i] > start[i] {
                bmins[i] = start[i] + mins[i] - 1.0;
                bmaxs[i] = end[i] + maxs[i] + 1.0;
            } else {
                bmins[i] = end[i] + mins[i] - 1.0;
                bmaxs[i] = start[i] + maxs[i] + 1.0;
            }
        }
        let pass_owner = if pass != ENTITYNUM_NONE && (pass as usize) < MAX_GENTITIES {
            let o = self.ents[pass as usize].owner;
            if o == ENTITYNUM_NONE { -1 } else { o }
        } else {
            -1
        };
        for i in self.entities_in_box(&bmins, &bmaxs) {
            if tr.allsolid {
                break;
            }
            if unlinked(i) {
                continue;
            }
            let e = &self.ents[i];
            if pass != ENTITYNUM_NONE && (i as i32 == pass || e.owner == pass || e.owner == pass_owner) {
                continue;
            }
            if mask & e.contents == 0 {
                continue;
            }
            let mut t = if e.bmodel {
                self.cm.transformed_box_trace(start, end, mins, maxs, e.model_index.max(0) as usize, mask, &e.origin, &e.angles)
            } else {
                cm::trace_against_box(start, end, mins, maxs, &e.mins, &e.maxs, &e.origin, e.contents, mask)
            };
            if t.allsolid {
                tr.allsolid = true;
                t.entity_num = i as i32;
            } else if t.startsolid {
                tr.startsolid = true;
                t.entity_num = i as i32;
            }
            if t.fraction < tr.fraction {
                let old_start = tr.startsolid;
                t.entity_num = i as i32;
                tr = t;
                tr.startsolid |= old_start;
            }
        }
        tr
    }

    /// `CG_BiSphereTrace`: world, then every solid entity (CG_ClipMoveToEntities
    /// with TT_BISPHERE; bmodels are offset but never rotated).
    pub fn bisphere_trace(&self, start: &Vec3, end: &Vec3, start_rad: f32, end_rad: f32, skip: i32, mask: i32) -> Trace {
        let mut tr = self.cm.bisphere_trace(start, end, start_rad, end_rad, 0, mask);
        tr.entity_num = if tr.fraction != 1.0 { ENTITYNUM_WORLD } else { ENTITYNUM_NONE };
        for i in 0..self.num_entities {
            let e = &self.ents[i];
            if !e.inuse || !e.linked || e.contents == 0 || i as i32 == skip {
                continue;
            }
            let mut t = if e.bmodel {
                let s = sub(start, &e.origin);
                let en = sub(end, &e.origin);
                let mut t = self.cm.bisphere_trace(&s, &en, start_rad, end_rad, e.model_index.max(0) as usize, mask);
                for k in 0..3 {
                    t.endpos[k] = start[k] + t.fraction * (end[k] - start[k]);
                }
                t
            } else {
                cm::bisphere_against_box(start, end, start_rad, end_rad, &e.mins, &e.maxs, &e.origin, e.contents, mask)
            };
            if t.allsolid || t.fraction < tr.fraction {
                t.entity_num = i as i32;
                if tr.lateral_fraction < t.lateral_fraction {
                    let old = tr.lateral_fraction;
                    tr = t;
                    tr.lateral_fraction = old;
                } else {
                    tr = t;
                }
            } else if t.startsolid {
                tr.startsolid = true;
            }
            if tr.allsolid {
                break;
            }
        }
        tr
    }

    /// `SV_PointContents`.
    pub fn point_contents(&self, p: &Vec3, pass: i32) -> i32 {
        self.point_contents_ex(p, pass, &|_| false)
    }

    pub fn point_contents_ex(&self, p: &Vec3, pass: i32, unlinked: &dyn Fn(usize) -> bool) -> i32 {
        let mut c = self.cm.point_contents(p);
        for i in self.entities_in_box(p, p) {
            if i as i32 == pass || unlinked(i) {
                continue;
            }
            let e = &self.ents[i];
            if e.bmodel {
                c |= self.cm.transformed_point_contents(p, e.model_index.max(0) as usize, &e.origin, &e.angles);
            } else {
                c |= e.contents;
            }
        }
        c
    }

    /// `SV_EntityContact`: does the box touch the entity's actual shape?
    pub fn entity_contact(&self, mins: &Vec3, maxs: &Vec3, i: usize) -> bool {
        let e = &self.ents[i];
        let zero = [0.0f32; 3];
        let t = if e.bmodel {
            self.cm.transformed_box_trace(&zero, &zero, mins, maxs, e.model_index.max(0) as usize, -1, &e.origin, &e.angles)
        } else {
            cm::trace_against_box(&zero, &zero, mins, maxs, &e.mins, &e.maxs, &e.origin, e.contents, -1)
        };
        t.startsolid
    }

    /// `G_Visible`-style line of sight.
    pub fn visible(&self, a: usize, b: usize, mask: i32) -> bool {
        let tr = self.trace(&self.ents[a].origin, None, None, &self.ents[b].origin, a as i32, mask);
        tr.entity_num == b as i32 || tr.fraction == 1.0
    }

    // -------------------------------------------------------------- spawning
    fn spawn_map_entities(&mut self) {
        let list: Vec<(BuildableId, Vec3, f32)> = self.spots.iter().filter(|s| s.kind == EntityKind::Buildable).filter_map(|s| s.buildable.map(|b| (b, s.origin, s.yaw))).collect();
        for (b, o, yaw) in list {
            super::buildable::spawn_from_map(self, b, o, [0.0, yaw, 0.0]);
        }
    }

    pub fn intermission_point(&self, team: Team) -> (Vec3, Vec3) {
        let want = if team == Team::None { EntityKind::PlayerStart } else { EntityKind::Intermission };
        if let Some(s) = self.spots.iter().find(|s| s.kind == want && (team == Team::None || s.team == team)) {
            return (s.origin, [0.0, s.yaw, 0.0]);
        }
        // G_SelectSpectatorSpawnPoint falls back to the player start
        if let Some(s) = self.spots.iter().find(|s| s.kind == EntityKind::PlayerStart) {
            return (s.origin, [0.0, s.yaw, 0.0]);
        }
        ([0.0, 0.0, 256.0], [0.0; 3])
    }

    // ----------------------------------------------------------- the frame
    /// `G_RunFrame(levelTime)`: one fixed step of `msec`.
    pub fn frame(&mut self, msec: i32) {
        // the game is paused (admin pause): level.time stands still
        if self.level.paused_time != 0 {
            self.level.paused_time += msec;
            self.level.ptime3000 += msec;
            let t = self.level.time;
            while self.level.ptime3000 > 3000 {
                self.level.ptime3000 -= 3000;
                self.center_prints.push((t, None, None, "The game has been paused. Please wait.".into()));
                let p = self.level.paused_time;
                if (110000..=119000).contains(&p) {
                    self.prints.push((t, None, None, format!("Server: Game will auto-unpause in {} seconds", ((120000 - p) as f32 / 1000.0) as i32)));
                }
            }
            // prevents clients from getting lagged-out messages
            for c in self.clients.iter_mut().filter(|c| c.connected) {
                c.ps.command_time = t;
            }
            if self.level.paused_time > 120000 {
                self.prints.push((t, None, None, "Server: The game has been unpaused automatically (2 minute max)".into()));
                self.center_prints.push((t, None, None, "The game has been unpaused!".into()));
                self.level.paused_time = 0;
            }
            return;
        }
        self.level.framenum += 1;
        self.level.previous_time = self.level.time;
        self.level.time += msec;
        self.level.msec = msec;
        for s in crate::gen::systems::SYSTEMS.iter() {
            (s.run)(self);
        }
    }

    /// Players and bots: join / choose / commands (the client command layer).
    /// The single-process path: the first human is the host's own player.
    pub fn connect(&mut self, name: &str, bot: bool) -> Option<usize> {
        self.connect_ex(None, &ConnectInfo { name: name.to_string(), use_unlagged: true, ..Default::default() }, bot).ok()
    }

    /// `ClientConnect` + `ClientBegin` for `slot` (None: the first free one).
    /// A networked client brings its cl_guid and address in `info`
    /// (dec033): the GUID must be 32 hex digits, a duplicate GUID drops a
    /// lagging ghost or refuses the newcomer, g_password and the admin bans
    /// (GUID or address) are checked. Err is the refusal reason.
    pub fn connect_ex(&mut self, slot: Option<usize>, info: &ConnectInfo, bot: bool) -> Result<usize, String> {
        let name = info.name.as_str();
        let i = match slot {
            Some(s) if s < MAX_CLIENTS && !self.clients[s].connected => s,
            Some(_) => return Err("Server is full".into()),
            None => (0..MAX_CLIENTS).find(|&i| !self.clients[i].connected).ok_or_else(|| "Server is full".to_string())?,
        };
        let networked = !bot && !info.ip.is_empty();
        // ClientConnect: pers.guid / pers.localClient / pers.ip / pers.admin, then the ban check
        let (guid, local) = if bot {
            (super::admin::bot_guid(name), false)
        } else if networked {
            (info.guid.clone(), info.ip == "localhost")
        } else {
            // one local client: the host's player (the first human to join)
            let local = !self.clients.iter().any(|c| c.connected && c.local_client);
            (if local { super::admin::LOCAL_GUID.to_string() } else { format!("{}{i}", super::admin::LOCAL_GUID) }, local)
        };
        let (_, ip) = super::admin::address_parse(&info.ip);
        self.clients[i] = Client { connected: true, is_bot: bot, local_client: local, ip, async_cmds: networked, use_unlagged: info.use_unlagged, team_info: info.team_info as i32, ..Default::default() };
        self.clients[i].admin = super::admin::admin_by_guid(self, &guid);
        self.clients[i].guid = guid.clone();
        let refuse = |w: &mut World, why: String| -> Result<usize, String> {
            w.clients[i] = Client::default();
            Err(why)
        };
        if let Some(reason) = super::admin::ban_check(self, i) {
            self.console.push_str(&format!("ClientConnect: {name} refused: {reason}\n"));
            return refuse(self, reason);
        }
        if networked {
            // check for a password
            let pw = self.level.password.clone();
            if !pw.is_empty() && !pw.eq_ignore_ascii_case("none") && pw != info.password {
                return refuse(self, "Invalid password".into());
            }
            // the guid: 32 hex digits
            if guid.len() != 32 || !guid.bytes().all(|b| b.is_ascii_hexdigit()) {
                return refuse(self, "Invalid GUID".into());
            }
            for j in 0..MAX_CLIENTS {
                if j == i || !self.clients[j].connected || !self.clients[j].guid.eq_ignore_ascii_case(&guid) {
                    continue;
                }
                if !super::client::client_is_lagging(self, j) {
                    self.center_prints.push((self.level.time, None, Some(j), "Your GUID is not secure".into()));
                    return refuse(self, "Duplicate GUID".into());
                }
                self.drops.push((j, "Ghost".into()));
            }
        }
        self.clients[i].inactivity_time = self.level.time + self.level.inactivity * 1000;
        self.clients[i].spectator_time = self.level.time;
        super::admin::namelog_connect(self, i);
        super::client::userinfo_changed(self, i, name, false);
        let (n, g, ipstr) = (self.clients[i].name.clone(), self.clients[i].guid.clone(), self.clients[i].ip.str.clone());
        super::log::log_printf(self, &format!("ClientConnect: {i} [{ipstr}] ({g}) \"{n}^7\" \"{}{n}{}^7\"\n", super::cmds::DECOLOR_OFF as char, super::cmds::DECOLOR_ON as char));
        if networked {
            self.prints.push((self.level.time, None, None, format!("{n}^7 connected")));
        }
        // the host's player is the top admin by default (dec026)
        if self.clients[i].local_client && self.clients[i].admin.is_none() {
            super::admin::host_admin(self, i);
        }
        let c = &mut self.clients[i];
        c.ps.client_num = i as i32;
        c.spectator = SpectatorState::Free as i32;
        c.spectator_client = -1;
        c.sticky_spec = true;
        c.enter_time = self.level.time;
        c.ps.persistant[crate::gen::enums::Pers::Specstate as usize] = SpectatorState::Free as i32;
        let e = &mut self.ents[i];
        *e = Entity { inuse: true, client: true, etype: EType::Player, classname: "player", owner: ENTITYNUM_NONE, ground_entity: ENTITYNUM_NONE, credits: vec![0; MAX_CLIENTS], ..Default::default() };
        let (o, a) = super::client::select_spectator_spawn_point(self);
        self.clients[i].ps.origin = o;
        self.clients[i].ps.viewangles = a;
        self.clients[i].ps.pm_type = crate::gen::enums::PmType::Spectator as i32;
        self.clients[i].ps.stats[crate::gen::enums::Stat::Health as usize] = 100;
        self.clients[i].ps.gravity = self.level.g_gravity as i32;
        self.clients[i].ps.speed = 600;
        self.clients[i].ps.grapple_point = [0.0, 0.0, 1.0];
        if self.level.intermission_time != 0 {
            super::client::move_client_to_intermission(self, i);
        }
        // ClientBegin
        if self.clients[i].admin.is_some() {
            super::log::admin_authlog(self, i);
        }
        super::log::log_printf(self, &format!("ClientBegin: {i}\n"));
        if networked {
            let n = self.clients[i].name.clone();
            self.prints.push((self.level.time, None, None, format!("{n}^7 entered the game")));
        }
        Ok(i)
    }

    pub fn disconnect(&mut self, i: usize) {
        let (n, g, ipstr) = (self.clients[i].name.clone(), self.clients[i].guid.clone(), self.clients[i].ip.str.clone());
        super::log::log_printf(self, &format!("ClientDisconnect: {i} [{ipstr}] ({g}) \"{n}^7\"\n"));
        super::client::leave_team(self, i);
        super::admin::namelog_disconnect(self, i);
        self.clients[i].connected = false;
        self.ents[i].inuse = false;
        self.ents[i].linked = false;
    }

    /// Integer credit shown as evos for aliens.
    pub fn display_funds(&self, i: usize) -> i32 {
        let c = &self.clients[i];
        if c.team == Team::Aliens {
            c.credit / k::ALIEN_CREDITS_PER_KILL
        } else {
            c.credit
        }
    }

    pub fn reactor(&self) -> Option<usize> {
        self.find_buildable(BuildableId::Reactor)
    }
    pub fn overmind(&self) -> Option<usize> {
        self.find_buildable(BuildableId::Overmind)
    }
    /// `G_FindBuildable` restricted to spawned, live ones (G_Reactor/G_Overmind).
    pub fn find_buildable(&self, b: BuildableId) -> Option<usize> {
        (MAX_CLIENTS..self.num_entities).find(|&i| {
            let e = &self.ents[i];
            e.inuse && e.etype == EType::Buildable && e.buildable == Some(b) && e.spawned && e.health > 0
        })
    }

    pub fn link(&mut self, i: usize) {
        self.ents[i].linked = true;
        if self.ents[i].bmodel || self.ents[i].etype == EType::ModelDoor {
            let (lo, hi) = self.ents[i].abs_bounds();
            let (mut a1, mut a2) = (-1, -1);
            for l in self.cm.box_leafnums(&lo, &hi) {
                let area = self.cm.leafs[l as usize].area;
                if area != -1 {
                    if a1 != -1 && a1 != area {
                        a2 = area;
                    } else {
                        a1 = area;
                    }
                }
            }
            self.ents[i].areanum = a1;
            self.ents[i].areanum2 = a2;
        }
    }

    /// `SV_AdjustAreaPortalState`.
    pub fn adjust_area_portal_state(&mut self, i: usize, open: bool) {
        let (a1, a2) = (self.ents[i].areanum, self.ents[i].areanum2);
        if a2 == -1 || a1 == a2 {
            return;
        }
        self.cm.adjust_area_portal_state(a1, a2, open);
    }

    /// `trap_InPVS`.
    pub fn in_pvs(&self, a: &Vec3, b: &Vec3) -> bool {
        self.cm.in_pvs(a, b)
    }
    pub fn unlink(&mut self, i: usize) {
        self.ents[i].linked = false;
    }
    pub fn set_origin(&mut self, i: usize, o: Vec3) {
        let e = &mut self.ents[i];
        e.pos = Trajectory { tr_type: crate::gen::enums::Traj::Stationary as i32, tr_time: 0, tr_duration: 0, tr_base: o, tr_delta: [0.0; 3] };
        e.origin = o;
    }

    pub fn on_same_team(&self, a: usize, b: usize) -> bool {
        let ta = self.team_of(a);
        let tb = self.team_of(b);
        ta != Team::None && ta == tb && self.ents[a].client && self.ents[b].client
    }
    pub fn team_of(&self, i: usize) -> Team {
        if i < MAX_CLIENTS && self.ents[i].client {
            self.clients[i].team
        } else {
            self.ents[i].team
        }
    }
}

/// `run_entities` system: the entity loop of G_RunFrame.
pub fn run_entities(w: &mut World) {
    let msec = w.level.msec;
    let t = w.level.time;
    for i in 0..w.num_entities {
        if !w.ents[i].inuse {
            continue;
        }
        if w.ents[i].free_after_event && t - w.ents[i].event_time > k::EVENT_VALID_MSEC {
            w.free(i);
            continue;
        }
        if w.ents[i].free_after_event {
            continue;
        }
        // calculate the acceleration of this entity
        if w.ents[i].evaluate_acceleration && msec > 0 {
            let e = &mut w.ents[i];
            let dv = sub(&e.pos.tr_delta, &e.old_velocity);
            e.acceleration = scale(&dv, 1.0 / msec as f32);
            let da = sub(&e.acceleration, &e.old_accel);
            e.jerk = scale(&da, 1.0 / msec as f32);
            e.old_velocity = e.pos.tr_delta;
            e.old_accel = e.acceleration;
        }
        match w.ents[i].etype {
            EType::Missile => super::missile::run_missile(w, i),
            EType::Buildable => super::buildable::buildable_think(w, i, msec),
            EType::Corpse => super::physics::physics(w, i, msec),
            EType::Mover => super::mover::run_mover(w, i),
            _ => {
                if i >= MAX_CLIENTS {
                    run_think(w, i);
                }
            }
        }
    }
}

/// `G_RunThink`.
pub fn run_think(w: &mut World, i: usize) {
    let nt = w.ents[i].nextthink;
    if nt <= 0 || nt > w.level.time {
        return;
    }
    w.ents[i].nextthink = 0;
    if let Some(f) = w.ents[i].think {
        f(w, i);
    }
}

impl PmWorld for World {
    fn trace(&self, start: &Vec3, mins: Option<&Vec3>, maxs: Option<&Vec3>, end: &Vec3, pass: i32, mask: i32) -> Trace {
        World::trace(self, start, mins, maxs, end, pass, mask)
    }
    fn point_contents(&self, p: &Vec3, pass: i32) -> i32 {
        World::point_contents(self, p, pass)
    }
}

