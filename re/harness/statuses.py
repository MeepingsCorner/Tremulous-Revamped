#!/usr/bin/env python3
"""Recompute sheets/re/symbols.tsv `status` from the port (MDD 7.3 symbol map).

Rules (re-run after any kernel change; idempotent):
  Verified  bg_pmove.c / bg_slidemove.c / cm_trace.c AABB path: covered by the
            golden-trace parity test against the C oracle (re/traces/*.sha256);
            cm_patch.c / cm_polylib.c / cm_test.c: real-BSP query hashes against
            re/harness/cm_ref (re/traces/cm/*.sha256, tests/patch.rs).
  Done      a symbol in MAP (ported under another name; this also lifts an
            extracted Replaced), or
  Done      a Rust item names the C function in a doc comment (`G_Damage`), or a
            Rust fn has the same name modulo case/underscores/prefix (bg_, g_, cm_, pm_).
  Done      cgame/ui symbols named in a crates/trem_godot doc comment (else Replaced).
  Dropped   capsule/sphere collision code (dec014, cm.rs header).
  Done      TT_BISPHERE + brush edges (CM_BiSphereTrace, CMod_CreateBrushSideWindings).
  unchanged otherwise (Todo / Replaced / Dropped from extraction).
"""
import re, pathlib, sys
root = pathlib.Path(__file__).resolve().parents[2]
kernel = root / "crates/trem_sim/src/kernel"
doc, fns = set(), set()
for f in kernel.glob("*.rs"):
    src = f.read_text()
    doc |= {m.lower() for m in re.findall(r"`([A-Za-z_][A-Za-z0-9_]*)`", src)}
    fns |= {m.replace("_", "").lower() for m in re.findall(r"\bfn\s+([a-z_][a-z0-9_]*)", src)}
# cgame / ui are ported into the presentation crate (dec021, dec023, dec025):
# a symbol named in a trem_godot doc comment is Done there
gdoc = set()
for f in (root / "crates/trem_godot/src").glob("*.rs"):
    gdoc |= {m.lower() for m in re.findall(r"`([A-Za-z_][A-Za-z0-9_]*)`", f.read_text())}
VERIFIED_FILES = ("game/bg_pmove.c", "game/bg_slidemove.c", "qcommon/cm_trace.c", "qcommon/cm_patch.c", "qcommon/cm_polylib.c", "qcommon/cm_test.c")
DROP = re.compile(r"capsule|cylinder|(?<!bi)sphere|projectpoint|distancefromline|vectordistancesquared|squarerootfloat|writeareabits|drawdebug|debugsurface")
# TT_BISPHERE + lateral fraction (light flare REALFADE): ported, not oracle-hashed
PORTED_TRACE = re.compile(r"bisphere|proximityto")
# PVS / area portals are ported (cm.rs load_visibility .. in_pvs) but not oracle-hashed
PORTED_CM = re.compile(r"area|pvs|cluster")

# C symbols ported under another name: symbol -> Rust item (must exist).
MAP = {
    "getclientmass": "client::client_mass", "clientintermissionthink": "client::client_think_real",
    "clientthink": "client::think_all", "g_runclient": "client::think_all",
    "hmgturret_checktarget": "buildable::turret_check_target", "hmgturret_findenemy": "buildable::h_mg_turret_think",
    "g_comparebuildablesforremoval": "buildable::sufficient_bp_available", "g_setbuildablelinkstate": "buildable::can_build",
    "g_setbuildablemarkedlinkstate": "buildable::can_build", "g_spawnbuildablethink": "buildable::spawn_from_map",
    "g_spawnbuildable": "buildable::spawn_from_map", "nulldiefunction": "buildable::null_die",
    "sp_info_player_deathmatch": "spawn::info_player_deathmatch", "sp_info_alien_intermission": "spawn::info_intermission",
    "sp_info_human_intermission": "spawn::info_intermission",
    "clientconnect": "world::World::connect", "clientbegin": "world::World::connect", "clientdisconnect": "world::World::disconnect",
    "g_initgame": "world::World::from_bsp", "g_runframe": "world::World::frame", "logexit": "log::log_exit",
    "g_initspawnqueue": "client::queue", "g_getspawnqueuelength": "client::queue", "g_popspawnqueue": "client::spawn_queues",
    "g_peekspawnqueue": "client::spawn_queues", "g_searchspawnqueue": "client::search_queue", "g_pushspawnqueue": "client::push_queue",
    "g_removefromspawnqueue": "client::remove_from_queue", "g_getposinspawnqueue": "client::queue_pos", "g_spawnclients": "client::spawn_queues",
    "g_evaluateacceleration": "world::run_entities",
    "sp_toggle_particle_system": "trigger::use_particle_system", "sp_use_particle_system": "trigger::use_particle_system",
    "sp_use_anim_model": "trigger::use_anim_model", "sp_use_light_flare": "trigger::use_light_flare",
    "fire_flamer": "missile::fire", "fire_blaster": "missile::fire", "fire_pulserifle": "missile::fire", "fire_lucifercannon": "missile::fire",
    "launch_grenade": "missile::fire", "fire_hive": "missile::fire", "fire_lockblob": "missile::fire", "fire_slowblob": "missile::fire",
    "fire_paralockblob": "missile::fire", "fire_bounceball": "missile::fire",
    "lockbloblauncherfire": "weapon::fire_weapon", "hivefire": "weapon::fire_weapon", "blasterfire": "weapon::fire_weapon",
    "pulseriflefire": "weapon::fire_weapon", "throwgrenade": "weapon::fire_weapon", "slowblobfire": "weapon::fire_weapon",
    "bounceballfire": "weapon::fire_weapon", "g_updatezapeffect": "weapon::update_zaps",
    "g_transposematrix": "mover::transpose", "manualdoortriggerspectator": "mover::manual_trigger_spectator",
    "g_addspawnvartoken": "bsp::Bsp::entity_list", "g_parsespawnvars": "bsp::Bsp::entity_list", "g_newstring": "spawn::parse_field",
    "g_teamfromstring": "client::cmd_team", "g_teamcommand": "vote::check_vote",
    "hurt_use": "trigger::toggle_link_use", "trigger_gravity_use": "trigger::toggle_link_use", "trigger_heal_use": "trigger::toggle_link_use",
    "trigger_win": "trigger::fire_trigger_win",
    "trigger_buildable_match": "trigger::trigger_buildable_touch", "trigger_buildable_trigger": "trigger::trigger_buildable_touch",
    "trigger_buildable_use": "trigger::toggle_nodraw_use", "trigger_class_match": "trigger::trigger_class_touch",
    "trigger_class_trigger": "trigger::trigger_class_touch", "trigger_class_use": "trigger::toggle_nodraw_use",
    "trigger_equipment_match": "trigger::trigger_equipment_touch", "trigger_equipment_trigger": "trigger::trigger_equipment_touch",
    "trigger_equipment_use": "trigger::toggle_nodraw_use",
    "g_findconfigstringindex": "world::World::sound_index", "g_find": "spawn::find_targetname", "g_initgentity": "world::World::spawn",
    "g_entitiesfree": "world::World::spawn", "g_tempentity": "world::World::temp_event", "g_broadcastevent": "world::World::temp_event",
    "g_findradius": "world::World::entities_in_box", "g_closestent": "buildable::find_power_for",
    "g_bounceprojectile": "weapon::fire_weapon", "snapvectornormal": "weapon::fire_weapon",
    "bloodspurt": "weapon::fire_weapon", "widebloodspurt": "weapon::fire_weapon",
    "g_triggermenu": "client::notify", "g_triggermenuargs": "client::notify", "g_closemenus": "client::notify",
    "g_setclientsound": "client::end_frame", "lookatkiller": "combat::player_die",
    "bg_addupgradetoinventory": "bg::add_upgrade", "bg_removeupgradefrominventory": "bg::remove_upgrade",
    "setplanesignbits__cm_load": "cm::Plane::set_signbits", "cm_boundbrush": "bsp::Bsp::clip_map",
    "cm_inlinemodel": "cm::ClipMap::trace_model", "cm_numinlinemodels": "cm::ClipMap::trace_model",
    "cm_cliphandletomodel": "cm::ClipMap::trace_model", "cm_entitystring": "bsp::Bsp::entity_list",
    "cm_tempboxmodel": "cm::trace_against_box", "cm_initboxhull": "cm::trace_against_box",
    "cm_modelbounds": "mover::set_brush_model", "cm_clearmap": "bsp::Bsp::clip_map",
    # g_admin.c / g_namelog.c / g_svcmds.c / g_session.c / layouts (dec026)
    "g_admin_cmdlist": "admin::cmdlist", "admin_permission": "admin::admin_permission", "g_admin_cmd": "admin::builtin",
    "g_admin_level": "admin::level_def", "g_admin_admin": "admin::admin_by_guid", "g_admin_command": "admin::custom_command",
    "g_admin_permission": "admin::permission", "g_admin_name_check": "admin::name_check",
    "admin_higher_admin": "admin::higher_admin", "admin_higher_guid": "admin::higher_guid", "admin_higher": "admin::higher",
    "admin_writeconfig_string": "admin::writeconfig", "admin_writeconfig_int": "admin::writeconfig", "admin_writeconfig": "admin::writeconfig",
    "admin_readconfig_string": "admin::readconfig_string", "admin_readconfig_int": "admin::readconfig_int",
    "admin_default_levels": "admin::default_levels", "admin_match": "admin::listadmins", "admin_out": "admin::listadmins",
    "admin_listadmins": "admin::listadmins", "g_admin_duration": "admin::duration", "g_admin_ban_matches": "admin::ban_matches",
    "g_admin_match_ban": "admin::match_ban", "g_admin_ban_check": "admin::ban_check", "g_admin_cmd_check": "admin::cmd_check",
    "g_admin_readconfig": "admin::readconfig", "g_admin_parse_time": "admin::parse_time",
    "g_admin_time": "admin::cmd_time", "g_admin_setlevel": "admin::cmd_setlevel", "g_admin_kick": "admin::cmd_kick",
    "g_admin_ban": "admin::cmd_ban", "g_admin_unban": "admin::cmd_unban", "g_admin_adjustban": "admin::cmd_adjustban",
    "g_admin_putteam": "admin::cmd_putteam", "g_admin_changemap": "admin::cmd_changemap", "g_admin_mute": "admin::cmd_mute",
    "g_admin_denybuild": "admin::cmd_denybuild", "g_admin_listadmins": "admin::cmd_listadmins",
    "g_admin_listlayouts": "admin::cmd_listlayouts", "g_admin_listplayers": "admin::cmd_listplayers",
    "ban_matchname": "admin::cmd_showbans", "ban_out": "admin::cmd_showbans", "g_admin_showbans": "admin::cmd_showbans",
    "g_admin_adminhelp": "admin::cmd_adminhelp", "g_admin_admintest": "admin::cmd_admintest", "g_admin_allready": "admin::cmd_allready",
    "g_admin_endvote": "admin::cmd_endvote", "g_admin_spec999": "admin::cmd_spec999", "g_admin_rename": "admin::cmd_rename",
    "g_admin_restart": "admin::cmd_restart", "g_admin_nextmap": "admin::cmd_nextmap",
    "namelog_matchname": "admin::namelog_search", "namelog_out": "admin::namelog_search", "g_admin_namelog": "admin::cmd_namelog",
    "g_namelogfromstring": "admin::namelog_from_string", "g_admin_lock": "admin::cmd_lock", "g_admin_builder": "admin::cmd_builder",
    "g_admin_pause": "admin::cmd_pause", "g_admin_buildlog": "admin::cmd_buildlog", "g_admin_revert": "admin::cmd_revert",
    "g_admin_print": "admin::admp", "g_admin_buffer_begin": "admin::begin", "g_admin_buffer_end": "admin::end",
    "g_admin_buffer_print": "admin::print", "g_admin_cleanup": "admin::cleanup", "g_adminmessage": "cmds::admin_message",
    "g_namelog_cleanup": "admin::namelog_cleanup", "g_namelog_connect": "admin::namelog_connect",
    "g_namelog_disconnect": "admin::namelog_disconnect", "g_namelog_update_score": "admin::namelog_update_score",
    "g_namelog_update_name": "admin::namelog_update_name", "g_namelog_restore": "admin::namelog_restore",
    "g_writeclientsessiondata": "session::write_client_session_data", "g_readsessiondata": "session::read_session_data",
    "g_initsessiondata": "session::init_session_data", "g_writesessiondata": "session::write_session_data",
    "svcmd_entitylist_f": "svcmds::entity_list", "clientforstring": "svcmds::client_for_string", "svcmd_status_f": "svcmds::status",
    "svcmd_forceteam_f": "svcmds::force_team", "svcmd_layoutsave_f": "svcmds::layout_save", "svcmd_layoutload_f": "svcmds::layout_load",
    "svcmd_teamwin_f": "svcmds::team_win", "svcmd_teammessage_f": "svcmds::team_message", "svcmd_centerprint_f": "svcmds::center_print",
    "svcmd_ejectclient_f": "svcmds::eject_client", "svcmd_dumpuser_f": "svcmds::dump_user", "svcmd_pr_f": "svcmds::pr",
    "svcmd_printqueue_f": "svcmds::print_queue", "svcmd_messagewrapper": "svcmds::message_wrapper",
    "svcmd_listmapswrapper": "svcmds::list_maps", "consolecommand": "svcmds::console_command",
    "g_layoutsave": "layout::layout_save", "g_layoutlist": "layout::layout_list", "g_layoutselect": "layout::layout_select",
    "g_layoutbuilditem": "layout::layout_build_item", "g_layoutload": "layout::layout_load",
    "g_buildlognew": "layout::build_log_new", "g_buildlogset": "layout::build_log_set", "g_buildlogauto": "layout::build_log_auto",
    "g_buildlogrevertthink": "layout::build_log_revert_think", "g_buildlogrevert": "layout::build_log_revert",
    "llsort": "admin::readconfig", "cmplevel": "admin::readconfig", "concatargs": "cmds::concat",
    "g_clientcleanname": "client::clean_name",
    # dec034: the IP forms and the lag check
    "ban_matchip": "admin::cmd_showbans", "namelog_matchip": "admin::namelog_search", "g_clientislagging": "client::client_is_lagging",
    "bg_packentitynumbers": "svgame::pack_entity_numbers", "bg_unpackentitynumbers": "svgame::unpack_entity_numbers", "g_isemoticon": "client::is_emoticon", "clientinactivitytimer": "client::inactivity_timer",
    # wt-misc: the local player's userinfo, the engine's completion list, debug names
    "clientuserinfochanged": "client::userinfo_changed",
    "g_registercommands": "svcmds::register_commands", "g_unregistercommands": "svcmds::register_commands",
    "g_admin_register_cmds": "svcmds::register_commands", "g_admin_unregister_cmds": "svcmds::register_commands",
    "bg_eventname": "bg::event_name", "bg_teamname": "bg::team_name", "g_listcommands": "cmds::list_commands",
    # worldspawn's disabledEquipment / disabledClasses / disabledBuildables
    "bg_parsecsvequipmentlist": "bg::parse_csv_equipment_list", "bg_parsecsvclasslist": "bg::parse_csv_class_list",
    "bg_parsecsvbuildablelist": "bg::parse_csv_buildable_list", "bg_initallowedgameelements": "bg::init_allowed_game_elements",
    "bg_weaponisallowed": "bg::weapon_is_allowed", "bg_upgradeisallowed": "bg::upgrade_is_allowed",
    "bg_classisallowed": "bg::class_is_allowed", "bg_buildableisallowed": "bg::buildable_is_allowed",
}
# Replaced by data or by the presentation reading World directly.
REPLACED = {
    "g_parsedmgscript", "g_initdamagelocations", "g_nonsegmodel", "sendpendingpredictableevents",
    "vmmain", "g_printf", "g_error", "com_error", "com_printf", "g_registercvars", "g_updatecvars", "g_restorecvars", "checkcvars",
    "g_mapconfigs", "g_shutdowngame", "sortranks", "sendscoreboardmessagetoallclients", "g_printspawnqueue",
    "g_clientlistforteam", "g_updateteamconfigstrings", "teamplayinfomessage", "checkteamstatus", "buildshaderstateconfig",
    "tv", "vtos", "vectoyaw", "utilities", "g_spawnvector4",
    # bg_misc.c config parsing and lookups: the class/buildable/weapon/upgrade sheets + generated from_id
    "bg_buildablebyname", "bg_buildableconfig", "bg_buildableboundingbox", "bg_parsebuildablefile", "bg_initbuildableconfigs",
    "bg_classbyname", "bg_classconfig", "bg_parseclassfile", "bg_initclassconfigs", "bg_weaponbyname", "bg_upgradebyname",
    "cmdcmp", "atof_neg", "atoi_neg",
    "bg_playerstatetoentitystateextrapolate",
    # q_math.c helpers the ported code does not call (Rust std / qmath.rs cover the ones it does)
    "q_rand", "q_random", "clampchar", "clampshort", "colorbytes3", "colorbytes4", "normalizecolor", "rotatearounddirection",
    "axisclear", "axiscopy", "makenormalvectors", "vectorrotate", "q_rsqrt", "q_fabs", "lerpangle", "anglessubtract", "anglemod",
    "angledelta", "boundsintersectsphere", "boundsintersectpoint", "vectornormalize2", "_vectorma", "_dotproduct", "_vectorsubtract",
    "_vectoradd", "_vectorcopy", "_vectorscale", "vector4scale", "q_log2", "planetypefornormal", "matrixmultiply",
    "vectormatrixmultiply", "pointtolinedistance", "getperpendicularviewvector", "projectpointontovector", "vectormaxcomponent",
    "vectormincomponent", "q_isnan", "q_acos",
}
# Dropped with a decision (dec017, narrowed by dec026/dec027/dec034): no single-process equivalent.
DROPPED = {
    # (unlagged, G_ClientIsLagging and the IP forms are ported since dec034)
    "scoreistied", "g_selectivedamage",
    "cm_lumpchecksum", "cm_checksum",
    # (the completion list is ported: dec036; IP matching: dec034)
}
# networking (dec033, dec035): C symbols ported under another name
NET_MAP = {
    "cl_getgamestate": "cgview::init", "cl_getcurrentsnapshotnumber": "cl::get_snapshot", "cl_getparseentitystate": "cl::get_snapshot",
    "cl_getcurrentcmdnumber": "cgview::predict_player_state", "cl_getusercmd": "cgview::predict_player_state",
    "cl_configstringmodified": "cl::get_server_command", "cl_getfreeping": "cl::ping", "cl_getpingqueuecount": "cl::update_visible_pings",
    "cl_getpinginfo": "cl::get_ping", "cl_initserverinfo": "cl::server_info_packet", "cl_getserverstatus": "cl::server_status_response",
    "cl_parseserverinfo": "cl::system_info_changed", "cl_initdownloads": "cl::next_download", "cl_sendpurechecksums": "cl::cgame_loaded",
    "cl_netchan_process": "netchan::process", "cl_netchan_transmit": "netchan::transmit", "cl_netchan_transmitnextfragment": "netchan::transmit_next_fragment",
    "cl_reconnect_f": "nethost::connect", "cl_serverstatus_f": "nethost::command", "cl_shutdowncgame": "nethost::disconnect",
    "huff_getbloc": "huffman::compress", "huff_setbloc": "huffman::decompress", "msg_nuinithuffman": "huffman::msg_huff",
    "net_sendlooppacket": "netchan::send_packet", "net_ipsocket": "netchan::open", "net_config": "netchan::close", "netchan_init": "cl::new",
    "sys_stringtoadr": "netchan::string_to_adr", "sys_sockaddrtostring": "netchan::to_string_base", "sys_stringtosockaddr": "netchan::to_socket",
    "sv_addenttosnapshot": "sv::add_entities_visible_from_point", "sv_qsortentitynumbers": "sv::add_entities_visible_from_point",
    "sv_begindownload_f": "sv::execute_client_command", "sv_nextdownload_f": "sv::execute_client_command", "sv_stopdownload_f": "sv::execute_client_command",
    "sv_donedownload_f": "sv::execute_client_command", "sv_closedownload": "sv::execute_client_command", "sv_senddownloadmessages": "sv::write_download_to_client",
    "sv_disconnect_f": "sv::execute_client_command", "sv_updateuserinfo_f": "sv::execute_client_command", "sv_gamedropclient": "admin::drop_client",
    "sv_gamesendservercommand": "svgame::drain_server_commands", "sv_getserverinfo": "sv::serverinfo", "sv_killserver_f": "nethost::shutdown_server",
    "sv_serverinfo_f": "nethost::command", "sv_systeminfo_f": "nethost::command", "sv_setconfigstringrestrictions": "svgame::configstring_restricted",
    "sv_netchan_process": "netchan::process", "sv_netchan_transmitnextfragment": "netchan::transmit_next_fragment",
    "sv_netchan_transmitnextinqueue": "sv::send_queued_packets", "sv_netchan_freequeue": "sv::drop_client", "sv_sendqueuedmessages": "sv::send_queued_packets",
    "svc_bucketforaddress": "sv::rate_limit_address", "svc_hashforaddress": "sv::rate_limit_address", "sv_framemsec": "sv::frame_msec",
}
# the engine plumbing the hosts own (Godot / the dedicated example, std::net, Rust ownership, no VM)
NET_REPLACED = {
    "cl_shutdown", "cl_shutdownall", "cl_clearmemory", "cl_flushmemory", "cl_starthunkusers", "cl_initref", "cl_initrenderer", "cl_shutdownref",
    "cl_refmalloc", "cl_cm_loadmap", "cl_getglconfig", "cl_cgamesystemcalls", "cl_cgamerendering", "cl_addcgamecommand", "cl_gameconsoletext",
    "cl_cddialog", "cl_vid_restart_f", "cl_snd_restart_f", "cl_snd_shutdown", "cl_shutdowninput", "cl_joystickevent", "floatasint",
    "cl_setusercmdvalue", "in_mlookup", "cl_checkpaused",
    "msg_copy", "net_getlocaladdress", "net_addlocaladdress", "net_opensocks", "net_getcvars", "net_restart_f", "net_shutdown", "net_errorstring",
    "net_event", "net_sleep", "searchaddrinfo", "sys_showip",
    "sv_addoperatorcommands", "sv_removeoperatorcommands", "sv_areaentities", "sv_areaentities_r", "sv_clipmovetoentities", "sv_cliptoentity",
    "sv_cliphandleforentity", "sv_clearworld", "sv_createworldsector", "sv_sectorlist_f", "sv_unlinkentity", "sv_setbrushmodel", "sv_gentitynum",
    "sv_sventityforgentity", "sv_gentityforsventity", "sv_numforgentity", "sv_inpvsignoreportals", "sv_boundmaxclients", "sv_changemaxclients",
    "sv_clearserver", "sv_startup", "sv_initgameprogs", "sv_initgamevm", "sv_restartgameprogs", "sv_shutdowngameprogs", "sv_locategamedata",
    "sv_gamesystemcalls", "sv_touchcgame", "sv_freeclient", "sv_gameclientnum", "sv_getconfigstring", "sv_getusercmd", "sv_getuserinfo",
    "sv_setuserinfo", "sv_map_f", "sv_maprestart_f", "sv_checkpaused",
}
# not ported (dec035): mumble, update-server MOTD, debug dumps
# (demos, pure checksums, downloads and IPv6 multicast are ported since dec037, VoIP and video capture since dec038;
# the cgame's demo state traps, the demo name completion, the d1 demo loop
# and the vid_restart re-check of a pure client stay out)
NET_DROPPED = {
    "cl_motdpacket", "cl_requestmotd", "cl_oldgame", "cl_resetpureclientatserver", "cl_completercon", "cl_completeplayername", "cl_clientinfo_f",
    "cl_configstrings_f", "cl_showip_f", "cl_sayto_f", "cl_setmodel_f", "cl_updatemumble", "shownet",
    "msg_reportchangevectors_f",
    "sv_completemapname", "sv_expandnewlines", "sv_replacependingservercommands", "sv_mastergamestat",
    "cl_demostate", "cl_demopos", "cl_demoname", "cl_completedemoname", "cl_startdemoloop",
    "__attribute__",  # a GCC attribute line of cl_main.c the symbol scan took for a function
}
NET_DROP_RE = re.compile(r"voip|video")
path = root / "sheets/re/symbols.tsv"
out, counts = [], {}
for line in path.read_text().splitlines():
    if line.startswith("#") or line.startswith("id\t"):
        out.append(line); continue
    c = line.split("\t")
    sid, sub, ref, st = c[0], c[2], c[3], c[5]
    if sub in ("cgame", "ui"):
        st = "Done" if sid.split("__")[0] in gdoc else "Replaced"
    elif sub in ("net", "server", "client"):
        # networking (dec033): named in a kernel or presentation doc comment,
        # else the explicit Replaced / Dropped sets below
        base = sid.split("__")[0] or sid
        if base in NET_MAP:
            mod, _, item = NET_MAP[base].partition("::")
            if not re.search(r"\bfn\s+" + re.escape(item) + r"\b", (kernel / f"{mod}.rs").read_text()):
                sys.exit(f"statuses: NET_MAP {base} -> {NET_MAP[base]} not found")
            st = "Done"
        elif base in NET_REPLACED:
            st = "Replaced"
        elif base in NET_DROPPED or (NET_DROP_RE.search(base) and base not in doc and base not in gdoc):
            # dec038: VoIP and the video capture are ported; a demo /
            # record / voip / video symbol named in a doc comment is Done
            st = "Dropped"
        elif base in doc or base in gdoc or any(base.removeprefix(p).replace("_", "") in fns for p in ("cl_", "sv_", "svc_", "msg_", "net_", "netchan_", "huff_", "")):
            st = "Done"
        else:
            st = "Todo"
    elif st in ("Replaced",) and sid not in MAP:
        pass
    elif PORTED_TRACE.search(sid) and sid in doc:
        st = "Done"
    elif ref.startswith(VERIFIED_FILES) and not DROP.search(sid):
        st = "Verified"
    elif sub == "cm" and DROP.search(sid):
        st = "Dropped"
    elif sub == "cm" and PORTED_CM.search(sid) and sid in doc:
        st = "Done"
    elif sid in MAP:
        mod, _, item = MAP[sid].partition("::")
        name = item.split("::")[-1]
        src = (kernel / f"{mod}.rs").read_text()
        if not re.search(r"\bfn\s+" + re.escape(name) + r"\b", src):
            sys.exit(f"statuses: MAP {sid} -> {MAP[sid]} not found")
        st = "Done"
    elif sid in REPLACED:
        st = "Replaced"
    elif sid in DROPPED:
        st = "Dropped"
    elif ref.startswith("qcommon/cm_load.c") and sid.startswith("cmod_load"):
        st = "Done"
    elif sid in doc or any(sid.replace("_", "") == f or sid.removeprefix(p).replace("_", "") == f for f in fns for p in ("bg_", "g_", "cm_", "pm_")):
        st = "Done" if st in ("Todo", "Dropped") else st
    c[5] = st
    counts[(sub, st)] = counts.get((sub, st), 0) + 1
    out.append("\t".join(c))
path.write_text("\n".join(out) + "\n")
for k in sorted(counts):
    print(*k, counts[k], sep="\t")
