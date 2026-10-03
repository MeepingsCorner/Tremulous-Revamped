#!/usr/bin/env python3
"""Regenerate sheets/02-plan.tsv and sheets/03-impl.tsv (MDD 6.4 coverage).

Plan rows are what the port promises (one per class, weapon, upgrade, buildable,
system, map and cross-cutting feature); impl rows bind each to the Rust item
that delivers it (compile-checked by the `paths` generator), with status and
the evidence that backs it.
"""
import pathlib
root = pathlib.Path(__file__).resolve().parents[2]
S = root / "sheets"

def rows(name):
    lines = [l.rstrip("\n") for l in (S / "domain" / f"{name}.tsv").read_text().splitlines() if l and not l.startswith("#")]
    hdr = lines[0].split("\t")
    return [dict(zip(hdr, l.split("\t"))) for l in lines[1:]]

GOLDEN = "re/traces/arena.sha256"
GAME = "crates/trem_sim/tests/game.rs"
plan, impl = [], []
def add(sheet, id_, kind, prio, item, status, ref, ev, notes="NULL"):
    plan.append((sheet, id_, kind, str(prio), notes))
    impl.append((sheet, id_, item, status, ref, ev))

scen_classes = {r["class"] for r in rows("scenarios")}
for r in rows("classes"):
    v = r["id"] in scen_classes
    add("classes", r["id"], "ClassMovement", 1, "crate::kernel::pmove::Pmove", "Verified" if v else "Done", "game/bg_pmove.c:1", GOLDEN if v else GAME,
        "movement golden-traced" if v else "movement shares verified code paths")
wb = {r["id"]: r for r in rows("weapon_behaviors")}
for r in rows("weapons"):
    b = wb.get(r["id"])
    if r["id"] == "level0":
        add("weapons", "level0", "WeaponFire", 1, "crate::kernel::weapon::check_venom_attack", "Done", "game/g_weapon.c:847", GAME, "auto-bite, not a fire event")
    elif b:
        add("weapons", r["id"], "WeaponFire", 1, b["fire1"], b["status"], b["ref_src"], GAME)
for r in rows("upgrades"):
    item = {"ammo": "crate::kernel::client::give_max_ammo", "medkit": "crate::kernel::client::client_think_real",
            "gren": "crate::kernel::client::client_think_real", "jetpack": "crate::kernel::pmove::Pmove"}.get(r["id"], "crate::kernel::client::cmd_buy")
    add("upgrades", r["id"], "Upgrade", 2, item, "Done", "game/g_cmds.c:2073", GAME)
for r in rows("buildable_behaviors"):
    add("buildables", r["id"], "BuildableThink", 1, r["think"], r["status"], r["ref_src"], GAME)
for r in rows("entity_spawns"):
    add("entity_spawns", r["id"], "EntitySpawn", 2, r["spawn"], r["status"], r["ref_src"], "crates/trem_sim/tests/movers.rs")
for r in rows("client_commands"):
    add("client_commands", r["id"], "ClientCommand", 2, r["handler"], r["status"], r["ref_src"], GAME)
SERVER = "crates/trem_sim/tests/server.rs"
NET = "crates/trem_sim/tests/net.rs"
SESSION = "crates/trem_sim/tests/net_session.rs"
for r in rows("admin_commands"):
    add("admin_commands", r["id"], "AdminCommand", 3, r["handler"], r["status"], r["ref_src"], SERVER)
for r in rows("systems"):
    add("systems", r["id"], "System", 1, r["run"], r["status"], r["ref_src"], GAME)
add("maps", "arena", "Map", 1, "crate::kernel::cm::from_sheet", "Verified", "qcommon/cm_trace.c:1", GOLDEN)
features = [
    ("bsp_loader", "crate::kernel::bsp::parse", "Verified", "qcommon/cm_load.c:1", "crates/trem_sim/tests/patch.rs", "IBSP v46 + CM_LoadMap collision half (patches, inline models): hashes vs re/harness/cm_ref"),
    ("patch_collision", "crate::kernel::patch::generate", "Verified", "qcommon/cm_patch.c:1", "crates/trem_sim/tests/patch.rs", "240k queries on the 8 maps bit-identical to the original"),
    ("bsp_world", "crate::kernel::spawn::spawn_entities", "Done", "game/g_spawn.c:638", "crates/trem_sim/tests/movers.rs", "every classname through the entity_spawns sheet"),
    ("movers", "crate::kernel::mover::run_mover", "Done", "game/g_mover.c:438", "crates/trem_sim/tests/movers.rs", "pusher, doors, plats, buttons, trains, rotating/bobbing/pendulum"),
    ("triggers", "crate::kernel::trigger::touch_triggers", "Done", "game/g_active.c:314", "crates/trem_sim/tests/movers.rs", "g_trigger.c, g_target.c, g_misc.c"),
    ("bp_zones", "crate::kernel::team::calculate_build_points", "Done", "game/g_main.c:1111", GAME, "repeater build-point zones and queues"),
    ("decon_marking", "crate::kernel::client::cmd_deconstruct", "Done", "game/g_cmds.c:1850", GAME, "g_markDeconstruct 3: mark, replace on build"),
    ("spawn_queue", "crate::kernel::client::spawn_queues", "Done", "game/g_main.c:1016", GAME, "NULL"),
    ("evolve", "crate::kernel::client::cmd_class", "Done", "game/g_cmds.c:1658", GAME, "NULL"),
    ("armoury", "crate::kernel::client::cmd_buy", "Done", "game/g_cmds.c:2073", GAME, "buy/sell"),
    ("build", "crate::kernel::client::cmd_build", "Done", "game/g_cmds.c:2405", GAME, "blueprint + build_fire"),
    ("stages", "crate::kernel::team::calculate_stages", "Done", "game/g_main.c:1266", GAME, "NULL"),
    ("locational_damage", "crate::kernel::combat::damage", "Done", "game/g_combat.c:1", GAME, "damage_regions sheet"),
    ("spectator_follow", "crate::kernel::client::follow_new_client", "Done", "game/g_cmds.c:2545", GAME, "NULL"),
    ("intermission", "crate::kernel::client::begin_intermission", "Done", "game/g_main.c:1587", GAME, "ExitLevel loads g_nextMap, else advances the map rotation (dec022), else restarts"),
    ("votes", "crate::kernel::vote::check_votes", "Done", "game/g_main.c:2143", GAME, "kick/mute/denybuild/allowbuild run the admin commands (dec026)"),
    ("pvs_areas", "crate::kernel::cm::ClipMap", "Done", "qcommon/cm_test.c:320", "crates/trem_sim/tests/movers.rs", "NULL"),
    ("world_effects", "crate::kernel::client::end_frame", "Done", "game/g_active.c:100", GAME, "P_WorldEffects + G_SetClientSound"),
    ("location", "crate::kernel::team::get_location", "Done", "game/g_team.c:244", GAME, "no stock map places target_location"),
    ("chat", "crate::kernel::cmds::say_all", "Done", "game/g_cmds.c:897", GAME, "with G_CensorString / G_FloodLimited and admin chat (dec026)"),
    ("timelimit", "crate::kernel::team::check_exit_rules", "Done", "game/g_main.c:2033", GAME, "g_timelimit 0 by default in the sandbox"),
    ("map_rotation", "crate::kernel::maprotation::MapRotations", "Done", "game/g_maprotation.c:1097", "crates/trem_sim/src/kernel/maprotation.rs", "maprotation.cfg from the data folders; host keeps state (dec022)"),
    ("bisphere_trace", "crate::kernel::cm::bisphere_against_box", "Done", "qcommon/cm_trace.c:1680", "crates/trem_sim/tests/bsp.rs", "light flare REALFADE lateral fraction (dec024)"),
    ("admin", "crate::kernel::admin::cmd_check", "Done", "game/g_admin.c:944", SERVER, "host player is the top admin, GUID-only bans (dec026)"),
    ("namelog", "crate::kernel::admin::namelog_connect", "Done", "game/g_namelog.c:36", SERVER, "NULL"),
    ("server_console", "crate::kernel::svcmds::console_command", "Done", "game/g_svcmds.c:584", SERVER, "host console as a dedicated console (dec026)"),
    ("layouts", "crate::kernel::layout::layout_load", "Done", "game/g_buildable.c:4113", SERVER, "layouts/<map>/<name>.dat via user:// and the pk3s"),
    ("build_log", "crate::kernel::layout::build_log_revert", "Done", "game/g_buildable.c:4287", SERVER, "NULL"),
    ("name_cleaning", "crate::kernel::client::clean_name", "Done", "game/g_client.c:651", SERVER, "NULL"),
    ("inactivity", "crate::kernel::client::inactivity_timer", "Done", "game/g_active.c:513", SERVER, "g_inactivity 0 (off) by default"),
    ("flood_censor", "crate::kernel::cmds::flood_limited", "Done", "game/g_cmds.c:3049", SERVER, "g_censorship empty by default"),
    ("session", "crate::kernel::session::read_session_data", "Done", "game/g_session.c:69", SERVER, "host keeps per-slot session strings"),
    ("bots", "crate::kernel::bot::think_all", "Done", "NULL", "crates/trem_sim/tests/bots.rs", "new (dec015, dec028). Done when, 3 bots a side, on the sheet arena and the 8 official maps: in a 10-min match both teams kill >=3 enemies, deaths to the map <=10% of all deaths (0 on arena), no bot stuck >35 s, <=5 /kill, aliens evolve and humans buy, both teams damage the enemy's structures; and 2 bots of either team alone with the credit cap damage the enemy's structures (>=100 health) within 5 min, spending credits, <=2 map deaths, no bot stuck >35 s. Balance over 6 seeds per official map (dec036): neither team wins >67% of a map's runs or >60% of all runs, matches last >=480 s on average (>=360 s on every map). Evidence crates/trem_sim/tests/bots.rs (official maps: --ignored, release)"),
    ("warmup", "crate::kernel::team::warmup_over", "Done", "game/g_main.c:2473", GAME, "g_doWarmup 0 by default; SP_worldspawn's CS_WARMUP and the Warmup: log line, CG_DrawWarmup in the HUD"),
    ("userinfo_name", "crate::kernel::client::userinfo_changed", "Done", "game/g_client.c:813", SERVER, "the local player's name cvar at connect and on change (CL_CheckUserinfo), rename limits, namelog, ClientRename"),
    ("determinism", "crate::kernel::rng::Rng", "Verified", "NULL", "crates/trem_sim/tests/rng.rs", "seeded LCG shared with oracle (dec013)"),
    # networking (dec033 .. dec035)
    ("net_huffman", "crate::kernel::huffman::compress", "Verified", "qcommon/huffman.c:1", NET, "adaptive Huffman of the OOB connect and every netchan message: bit-exact vs re/harness/net_ref (re/traces/net.sha256)"),
    ("net_msg", "crate::kernel::msg::Msg", "Verified", "qcommon/msg.c:1", NET, "bit streams, entityState/playerState netField deltas, keyed usercmd deltas: bit-exact vs net_ref"),
    ("net_chan", "crate::kernel::netchan::Netchan", "Verified", "qcommon/net_chan.c:1", NET, "sequences, qport, fragments, loopback: bit-exact vs net_ref"),
    ("net_ip", "crate::kernel::netchan::Net", "Done", "qcommon/net_ip.c:1", NET, "non-blocking UDP per source, broadcast, NET_QueuePacket's packet delay"),
    ("server", "crate::kernel::sv::Server", "Done", "server/sv_main.c:1", SESSION, "connectionless queries, challenges, connect, gamestate, configstrings, rcon, downloads, timeouts, heartbeats"),
    ("snapshots", "crate::kernel::sv::write_area_bits", "Done", "server/sv_snapshot.c:1", SESSION, "PVS/area culled entity lists delta'd against the acked frame, rate control"),
    ("client", "crate::kernel::cl::NetClient", "Done", "client/cl_main.c:1", SESSION, "CA_* states, gamestate/snapshot parsing, usercmd packets, server time drift"),
    ("prediction", "crate::kernel::cgview::Cgame", "Done", "cgame/cg_predict.c:1", SESSION, "snapshot interpolation, Pmove prediction with error decay, events; within tolerance at 0 and 224 ms ping"),
    ("unlagged", "crate::kernel::unlagged::calc", "Done", "game/g_active.c:1", SESSION, "rewind of the other players to the shooter's command time (dec034)"),
    ("ip_bans", "crate::kernel::admin::address_compare", "Done", "game/g_admin.c:887", SERVER, "GUID or IP/CIDR bans, namelog addresses (dec034)"),
    ("server_browser", "crate::kernel::nethost::NetHost", "Done", "client/cl_main.c:4481", SESSION, "localservers broadcast, getservers to the masters, pings, featured labels; Godot ui via trap_LAN (dec035)"),
    ("guid", "crate::kernel::md5::md5_file", "Verified", "qcommon/md5.c:260", "crates/trem_sim/src/kernel/md5.rs", "cl_guid = MD5 of the qkey (RFC 1321 vectors)"),
    ("dedicated_server", "crate::kernel::nethost::server_cvars", "Done", "server/sv_init.c:661", "crates/trem_sim/examples/dedicated.rs", "cargo run -p trem_sim --example dedicated -- +map atcs; headless Godot: godot --headless --path godot -- +set dedicated 1 +map atcs (godot/tests/a_dl_shot.gd joins it, dec037)"),
    # dec037: pure servers, downloads, demos, IPv6
    ("pure_checksums", "crate::kernel::files::block_checksum", "Verified", "qcommon/files.c:1967", "crates/trem_sim/tests/pure.rs", "FS_LoadZipFile's checksum / pure_checksum over the zip CRCs with Com_BlockChecksum (md4.c): data-1.1.0.pk3 hashed against re/harness/pak_ref (re/traces/pak.sha256)"),
    ("pure_server", "crate::kernel::files::PakSet", "Done", "server/sv_client.c:970", "crates/trem_sim/tests/net_files.rs", "sv_pure 1: sv_paks / sv_referencedPaks, the client's restricted search path and cp reply, SV_VerifyPaks_f refusing a forged cp"),
    ("pk3_download", "crate::kernel::cl::ClDownload", "Done", "client/cl_main.c:2117", "crates/trem_sim/tests/net_files.rs", "CL_InitDownloads / CL_NextDownload (com_downloadPrompt, cl_allowDownload) / CL_ParseDownload into fs_homepath / CL_DownloadsComplete's FS_Restart and donedl against SV_WriteDownloadToClient's referenced paks; Godot: user://base, godot/tests/a_dl_shot.gd"),
    ("demos", "crate::kernel::demo::DemoState", "Done", "client/cl_main.c:688", "crates/trem_sim/tests/net_demo.rs", "record / stoprecord / demo / timedemo / cl_autoRecordDemo, demos/<name>.dm_71; playback gives back every recorded snapshot; Godot: user://demos, godot/tests/a_demo_shot.gd"),
    ("ipv6", "crate::kernel::netchan::set_net_enabled", "Done", "qcommon/net_ip.c:878", "crates/trem_sim/tests/net_ipv6.rs", "ipv6 sockets (IPV6_V6ONLY), net_enabled family priority, [addr]:port parsing and printing, NET_JoinMulticast6 LAN scan (socket tests skip without ipv6 on the host)"),
    # dec038: VoIP, video capture
    ("voip", "crate::kernel::voip::ClVoip", "Done", "client/cl_main.c:403", "crates/trem_sim/tests/voip.rs", "Opus voice over protocol 71 (clc/svc_voipOpus) with the bundled libopus 1.1; capture, targets, ignore/gain/muteall, VAD, meter; two clients over UDP at 35 dB SNR"),
    ("video_capture", "crate::kernel::avi::AviFile", "Done", "client/cl_avi.c:1", "crates/trem_sim/tests/avi.rs", "video / stopvideo: MJPEG or raw BGR AVI with PCM audio at cl_aviFrameRate; RIFF tree and idx1 checked, ffprobe when installed"),
    # dec039: the live master server
    ("master_list", "crate::kernel::cl::NetClient", "Done", "client/cl_main.c:4203", "crates/trem_sim/tests/net_master.rs", "getservers 71 full empty (master.tremulous.net reads the protocol first), getserversExt for ipv6 masters; heartbeat, getinfo challenge from the master's out port, list, ping, join against a darklegion tremulous-master stand-in"),
    # dec040: vanilla Tremulous 1.1.0 (protocol 69) servers
    ("vanilla_wire", "crate::kernel::v11net::com_hash_key", "Verified", "qcommon/msg.c:1", "crates/trem_sim/tests/net_v11.rs", "1.1.0's entityStateFields / playerStateFields deltas, Com_HashKey usercmd keys, CL_Netchan_Encode / CL_Netchan_Decode and the checksum-less netchan, hashed against re/harness/msg69_ref (re/source11 compiled natively, re/traces/msg69.sha256)"),
    ("vanilla_client", "crate::kernel::cl::NetClient", "Done", "client/cl_main.c:1", "crates/trem_sim/tests/net_v11.rs", "protocol 69 connections: getchallenge / challengeResponse / connect protocol 69, gamestate, snapshots kept as 1.1 delta bases and handed to the cgame through v11.rs, usercmds and commands through v11.rs, pure cp, UDP downloads, disconnect; end to end against a real 1.1.0 tremded (sv_pure 0 and 1)"),
    ("net_modes", "crate::kernel::nethost::parse_connect_args", "Done", "client/cl_main.c:1", "crates/trem_sim/tests/net_master.rs", "cl_netProtocol Auto (getinfo first, 71 without an answer) / 71 / 69, connect -1|-69|-71; the browser asks the masters for 71 and 69, merges the lists, takes 1.1.0 infoResponses without gamename, tags 1.1 / 1.3 servers, filters them (ui_browserProtocol) and joins each with its protocol"),
    # dec041: the game data of a Tremulous 1.1.0 (protocol 69) server
    ("vanilla_game_data", "crate::kernel::v11::ps_from_11", "Done", "game/bg_public.h:1", "crates/trem_sim/tests/net_v11.rs", "1.1.0 numbering (from the compiled 1.1.0 headers) and encodings of playerState, entities, events, configstrings, server and client commands, usercmds mapped to the legacy client; the hovel and the advanced construction kit drawn"),
    ("vanilla_prediction", "crate::kernel::pmove11::Pmove11", "Verified", "game/bg_pmove.c:1", "re/traces/arena11.sha256", "1.1.0's movement byte-exact against re/harness/pmove11_ref over scenarios11.tsv; the cgame predicts with it (no prediction misses against 1.1 movement)"),
]
for f in features:
    add("features", f[0], "Feature", 2, f[1], f[2], f[3], f[4], f[5])

def write(name, hdr_lines, cols, data):
    with open(S / name, "w") as fh:
        fh.write("\n".join(hdr_lines) + "\n" + "\t".join(cols) + "\n")
        for d in data:
            fh.write("\t".join(d) + "\n")
write("02-plan.tsv", ["# sheet: plan", "# version: 1", "# generator: none", "# key: sheet,id", "# owned_by: architect"], ["sheet", "id", "kind", "priority", "notes"], plan)
write("03-impl.tsv", ["# sheet: impl", "# version: 1", "# generator: paths", "# key: sheet,id", "# owned_by: verifier"], ["sheet", "id", "rust_item", "status", "ref_src", "evidence"], impl)
print(len(plan), "plan rows,", len(impl), "impl rows")
