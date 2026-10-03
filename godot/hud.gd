extends CanvasLayer
## HUD and menus. Pure presentation: keys and the mouse go to TremGame's
## client (Rust: the bind table of default.cfg / user://autogen.cfg, the
## cl_input.c buttons, cgame / ui console commands); the commands it cannot
## run itself (menus, console, scoreboard, the port's debug helpers) come
## back through the `host_command` signal. hud.gd handles Escape and the
## click that recaptures the mouse.

@onready var game = get_node("../Game")

var root: Control
var lbl_health: Label
var lbl_ammo: Label
var lbl_funds: Label
var lbl_feed: Label
var lbl_center: Label
var lbl_status: Label
var lbl_help: Label
var crosshair: Label
var damage_flash: ColorRect
var menu_panel: PanelContainer
var menu_box: VBoxContainer
var menu_title: Label
var score_panel: PanelContainer
var score_label: Label
var menu_kind := ""
## the original HUD (TremHud, Rust) draws health/ammo/credits/crosshair when
## the Tremulous data is present; these labels are then the fallback only
var orig_hud := false
var show_help := true
## the drop-down console (TremConsole, Rust: cl_console.c / cl_keys.c)
var console: Control
var lbl_vote: Label
var shown_intro := false
## the original in-game menus (TremMenus, Rust) when the Tremulous data is present
var trem_menus: Node

func _ready() -> void:
	if game.is_dedicated():
		# wt-a (dec037): a dedicated server (+set dedicated 1) has no ui
		queue_free()
		return
	game.host_command.connect(_on_host_command)
	root = Control.new()
	root.set_anchors_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(root)

	damage_flash = ColorRect.new()
	damage_flash.set_anchors_preset(Control.PRESET_FULL_RECT)
	damage_flash.color = Color(0.8, 0, 0, 0)
	damage_flash.mouse_filter = Control.MOUSE_FILTER_IGNORE
	root.add_child(damage_flash)

	crosshair = _label("+", 28, Control.PRESET_CENTER)
	crosshair.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	crosshair.position -= Vector2(8, 20)
	lbl_health = _label("", 30, Control.PRESET_BOTTOM_LEFT)
	lbl_health.position += Vector2(24, -64)
	lbl_ammo = _label("", 30, Control.PRESET_BOTTOM_RIGHT)
	lbl_ammo.position += Vector2(-360, -64)
	lbl_ammo.custom_minimum_size = Vector2(340, 0)
	lbl_ammo.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	lbl_funds = _label("", 18, Control.PRESET_TOP_LEFT)
	lbl_funds.position += Vector2(16, 12)
	lbl_feed = _label("", 16, Control.PRESET_TOP_RIGHT)
	lbl_feed.position += Vector2(-520, 12)
	lbl_feed.custom_minimum_size = Vector2(500, 0)
	lbl_feed.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	lbl_center = _label("", 26, Control.PRESET_CENTER_TOP)
	lbl_center.position += Vector2(-400, 140)
	lbl_center.custom_minimum_size = Vector2(800, 0)
	lbl_center.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	lbl_status = _label("", 18, Control.PRESET_CENTER_BOTTOM)
	lbl_status.position += Vector2(-400, -110)
	lbl_status.custom_minimum_size = Vector2(800, 0)
	lbl_status.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	lbl_help = _label("", 14, Control.PRESET_BOTTOM_LEFT)
	lbl_help.position += Vector2(24, -230)
	lbl_help.modulate = Color(1, 1, 1, 0.75)

	lbl_vote = _label("", 16, Control.PRESET_CENTER_LEFT)
	lbl_vote.position += Vector2(16, -120)
	lbl_vote.modulate = Color(1, 0.9, 0.4)
	# client console (wt-misc): the original's drop-down console with its edit
	# line, history and tab completion; Enter runs the line through the
	# client's command buffer (TremGame), text without a slash is chat in a game
	console = ClassDB.instantiate("TremConsole")
	root.add_child(console)
	console.game_path = console.get_path_to(game)

	menu_panel = PanelContainer.new()
	menu_panel.set_anchors_preset(Control.PRESET_CENTER)
	menu_panel.custom_minimum_size = Vector2(420, 0)
	menu_panel.position = Vector2(-210, -220)
	var vb := VBoxContainer.new()
	menu_panel.add_child(vb)
	menu_title = Label.new()
	menu_title.add_theme_font_size_override("font_size", 24)
	vb.add_child(menu_title)
	menu_box = VBoxContainer.new()
	vb.add_child(menu_box)
	root.add_child(menu_panel)
	menu_panel.hide()

	score_panel = PanelContainer.new()
	score_panel.set_anchors_preset(Control.PRESET_CENTER)
	score_panel.position = Vector2(-320, -240)
	score_panel.custom_minimum_size = Vector2(640, 0)
	score_label = Label.new()
	score_label.add_theme_font_size_override("font_size", 16)
	score_panel.add_child(score_label)
	root.add_child(score_panel)
	score_panel.hide()

	trem_menus = get_node_or_null("TremMenus")
	if trem_menus:
		trem_menus.menu_closed.connect(_on_trem_menu_closed)
		trem_menus.menu_opened.connect(_on_trem_menu_opened)
		trem_menus.ui_command.connect(_on_trem_ui_command)
	# no map yet (TremGame.start_in_main_menu): the original main menu
	# (UIMENU_MAIN, after the intro cinematic); else the team menu
	# (a map loading across frames opens the team menu once it is up)
	if game.is_loading():
		_team_menu_after_load = true
	elif not game.has_world() and trem_menus and trem_menus.has_menus():
		_open_main()
	else:
		open_menu("team")

## UI_SetActiveMenu( UIMENU_MAIN ) (TremMenus; the menu_opened signal
## follows when the intro cinematic plays first)
func _open_main() -> void:
	trem_menus.open("main")
	if trem_menus.is_open():
		menu_kind = "main"
		_capture(false)

func _label(text: String, size: int, preset: int) -> Label:
	var l := Label.new()
	l.text = text
	l.add_theme_font_size_override("font_size", size)
	l.add_theme_color_override("font_outline_color", Color.BLACK)
	l.add_theme_constant_override("outline_size", 6)
	l.set_anchors_preset(preset)
	l.mouse_filter = Control.MOUSE_FILTER_IGNORE
	root.add_child(l)
	return l

func _capture(on: bool) -> void:
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED if on else Input.MOUSE_MODE_VISIBLE
	game.input_enabled = on

## pk3 search path: user://base, <exe dir>/base, res://base, and --base=DIR.
func pk3_paths() -> PackedStringArray:
	var dirs: Array[String] = ["user://base", OS.get_executable_path().get_base_dir() + "/base", ProjectSettings.globalize_path("res://base")]
	for a in OS.get_cmdline_user_args() + OS.get_cmdline_args():
		if a.begins_with("--base="):
			dirs.append(a.substr(7))
	var out := PackedStringArray()
	for d in dirs:
		var da := DirAccess.open(d)
		if da == null:
			continue
		for f in da.get_files():
			if f.to_lower().ends_with(".pk3"):
				var full := ProjectSettings.globalize_path(d.path_join(f))
				if not out.has(full):
					out.append(full)
	return out

func map_items() -> Array:
	var items := [{"cmd": "@arena", "label": "Arena", "detail": "built-in sheet map", "enabled": true}]
	var paths := pk3_paths()
	for m in game.list_maps(paths):
		items.append({"cmd": "@map " + m, "label": m, "detail": "BSP from pk3", "enabled": true})
	for r in game.map_rotations():
		items.append({"cmd": "@rot " + r, "label": "Rotation: " + r, "detail": "maprotation.cfg", "enabled": true})
	items.append({"cmd": "@export", "label": "Export arena as pk3", "detail": "user://base/zz-arena.pk3", "enabled": true})
	return items

func open_menu(kind: String) -> void:
	if kind != "maps" and trem_menus and trem_menus.open(kind):
		menu_panel.hide()
		if trem_menus.is_open():
			menu_kind = kind
			_capture(false)
		return
	menu_kind = kind
	for c in menu_box.get_children():
		c.queue_free()
	var titles := {"team": "Choose a team", "spawn": "Spawn as", "evolve": "Evolve", "armoury": "Armoury", "build": "Build", "maps": "Maps (drop .pk3 files into user://base or --base=DIR)"}
	menu_title.text = titles.get(kind, kind)
	var items: Array = map_items() if kind == "maps" else game.menu_items(kind)
	for item in items:
		var b := Button.new()
		b.text = "%s    %s" % [item.label, item.detail]
		b.disabled = not item.enabled
		b.alignment = HORIZONTAL_ALIGNMENT_LEFT
		var cmd: String = item.cmd
		b.pressed.connect(func(): _menu_pick(cmd))
		menu_box.add_child(b)
	var close := Button.new()
	close.text = "Close  (Esc)"
	close.pressed.connect(close_menu)
	menu_box.add_child(close)
	menu_panel.show()
	_capture(false)

func close_menu() -> void:
	if trem_menus:
		trem_menus.close()
	menu_panel.hide()
	menu_kind = ""
	_capture(true)

func _on_trem_menu_closed() -> void:
	if menu_kind != "" and not menu_panel.visible:
		menu_kind = ""
		_capture(true)

func _on_trem_menu_opened(kind: String) -> void:
	if menu_kind == "":
		menu_kind = kind
		_capture(false)

## engine commands of the original menus (ingame_leave: uiScript quit / leave)
func _on_trem_ui_command(cmd: String) -> void:
	if cmd == "quit":
		get_tree().quit()
	elif cmd == "disconnect":
		# CL_Disconnect: the map goes, the ui shows the main menu (TremMenus);
		# also while connecting or loading a server's map (dec033)
		if game.has_world() or game.is_loading():
			game.shutdown_map()
		menu_panel.hide()
		_show_scores(false)
		for l in [lbl_center, lbl_status, lbl_feed, lbl_vote, lbl_health, lbl_ammo, lbl_funds]:
			l.text = ""
		if trem_menus and trem_menus.has_menus():
			menu_kind = "main"
			_capture(false)
		else:
			# without the original data there is no main menu: the map menu
			close_menu()
			open_menu("maps")
	elif cmd == "connected":
		# a map started from the main menu (createserver: map <name>)
		open_menu("team")

## the original menu for kind (ingame, say, say_team, ...) when loaded
func _open_trem(kind: String) -> bool:
	if trem_menus == null or not trem_menus.open(kind):
		return false
	menu_panel.hide()
	if trem_menus.is_open():
		menu_kind = kind
		_capture(false)
	return true

## Tab / intermission scoreboard: the original teamscore menu when available
func _show_scores(on: bool) -> void:
	if on and trem_menus and trem_menus.open("scores"):
		score_panel.hide()
		return
	if trem_menus:
		trem_menus.hide_scores()
	score_panel.visible = on

func _menu_pick(cmd: String) -> void:
	if cmd.begins_with("@"):
		var err2 := ""
		if cmd == "@arena":
			game.load_arena()
		elif cmd.begins_with("@rot "):
			if not game.start_map_rotation(cmd.substr(5)):
				err2 = "rotation failed"
		elif cmd == "@export":
			DirAccess.make_dir_recursive_absolute("user://base")
			err2 = game.export_arena_pk3(ProjectSettings.globalize_path("user://base/zz-arena.pk3"))
		else:
			# loads across frames behind the loading screen (CG_DrawLoadingScreen)
			err2 = game.load_map(pk3_paths(), cmd.substr(5))
		close_menu()
		if err2 != "":
			lbl_status.text = err2
		if cmd != "@export":
			_team_menu_after_load = true
		else:
			open_menu("maps")
		return
	var err: String = game.command(cmd)
	if err != "":
		lbl_status.text = err
	var kind := menu_kind
	close_menu()
	# chain: after choosing a team, choose a spawn item/class
	if kind == "team" and cmd != "team spectate":
		open_menu("spawn")
	elif kind == "armoury":
		open_menu("armoury")

## Con_ToggleConsole_f: the console takes the keys (Console_Key: Enter runs
## the line through the client's command buffer, Tab completes, Up/Down
## walk the history, PgUp/PgDn scroll)
func _open_console(prefix: String) -> void:
	console.open(prefix)
	game.input_enabled = false
	_capture(false)
	get_viewport().set_input_as_handled()

func _close_console() -> void:
	console.close()
	game.input_enabled = true
	_capture(true)

## Escape: the ui's (closes the console / a menu), else UI_SetActiveMenu(
## UIMENU_INGAME); without the original menus the team / spawn menu
func _escape() -> void:
	if console.is_down():
		_close_console()
	elif game.is_connecting():
		# CL_KeyDownEvent: Escape while connecting to a server or loading its
		# map disconnects (CL_Disconnect_f), then UIMENU_MAIN (dec035)
		game.shutdown_map()
		if trem_menus and trem_menus.has_menus():
			_open_main()
	elif not game.has_world() and trem_menus and trem_menus.has_menus():
		# disconnected: UIMENU_MAIN
		_open_main()
	elif menu_kind != "":
		close_menu()
	elif _open_trem("ingame"):
		pass
	else:
		var h: Dictionary = game.get_hud()
		if h.get("spectating", false) and h.get("team", "") != "Spectators":
			open_menu("spawn")
		else:
			open_menu("team")

## The client's commands for this side (TremGame.host_command)
func _on_host_command(line: String) -> void:
	var argv := line.split(" ", false)
	if argv.is_empty():
		return
	var arg := argv[1] if argv.size() > 1 else ""
	match argv[0].to_lower():
		"escape", "togglemenu":
			_escape()
		"toggleconsole":
			if console.is_down():
				_close_console()
			else:
				_open_console("")
		"messagemode":
			# without the original menus: the console, where text is chat
			if not _open_trem("say"):
				_open_console("")
		"messagemode2":
			if not _open_trem("say_team"):
				_open_console("/say_team ")
		"menu":
			if not _open_trem(arg):
				open_menu(arg)
		"closemenus":
			if menu_kind != "":
				close_menu()
		"+scores":
			_show_scores(true)
		"-scores":
			if not game.get_hud().get("intermission", false):
				_show_scores(false)
		"scoresup", "scoresdown":
			if trem_menus:
				trem_menus.scroll_scores(argv[0].to_lower() == "scoresdown")
		"quit":
			get_tree().quit()
		"disconnect":
			_on_trem_ui_command("disconnect")
		"screenshot", "screenshotjpeg":
			_screenshot(argv[0].to_lower() == "screenshotjpeg")
		"port_restart":
			close_menu()
			open_menu("team")
		"port_maps":
			close_menu()
			open_menu("maps")
		"port_help":
			show_help = not show_help
			lbl_help.text = ""

## R_ScreenShot_f: user://screenshots/shotNNNN.tga|jpg (png for tga here)
func _screenshot(jpeg: bool) -> void:
	await RenderingServer.frame_post_draw
	DirAccess.make_dir_recursive_absolute("user://screenshots")
	var img := get_viewport().get_texture().get_image()
	for n in 10000:
		var path := "user://screenshots/shot%04d.%s" % [n, "jpg" if jpeg else "png"]
		if not FileAccess.file_exists(path):
			if jpeg:
				img.save_jpg(path)
			else:
				img.save_png(path)
			lbl_status.text = "Wrote " + path
			return

## the console key (` ~) and Escape close the console while it has the keys
func _input(event: InputEvent) -> void:
	if not console.is_down() or not (event is InputEventKey) or not event.pressed or event.echo:
		return
	var k: int = event.keycode if event.keycode != KEY_NONE else event.physical_keycode
	if k == KEY_QUOTELEFT or k == KEY_ASCIITILDE or k == KEY_ESCAPE:
		_close_console()
		get_viewport().set_input_as_handled()

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo and not event.shift_pressed:
		var k: int = event.keycode if event.keycode != KEY_NONE else event.physical_keycode
		if k == KEY_ESCAPE:
			# escape is always handled special (CL_KeyDownEvent)
			_escape()
			get_viewport().set_input_as_handled()
	elif event is InputEventMouseButton and event.pressed and menu_kind == "" and not console.is_down() and Input.mouse_mode != Input.MOUSE_MODE_CAPTURED:
		# a click takes the mouse back (it is not a +attack)
		_capture(true)
		get_viewport().set_input_as_handled()

## The keys bound to the main commands (the bind table, user rebinds included)
func _help_text() -> String:
	var rows := [
		["+forward", "forward"], ["+back", "back"], ["+moveleft", "left"], ["+moveright", "right"],
		["+moveup", "jump"], ["+movedown", "crouch"], ["+speed", "walk"], ["+button8", "sprint"], ["+button6", "dodge"],
		["+attack", "fire"], ["+button5", "alt fire"], ["+button7", "use / evolve"], ["+button2", "use item"],
		["weapprev", "prev item"], ["weapnext", "next item"], ["reload", "reload"], ["buy ammo", "buy ammo"],
		["itemact medkit", "medkit"], ["deconstruct", "deconstruct"], ["messagemode", "chat"], ["messagemode2", "team chat"],
		["+scores", "scores"], ["vote yes", "vote yes"], ["vote no", "vote no"], ["toggleconsole", "console"],
		["toggle cg_thirdPerson", "3rd person"], ["port_addbot aliens", "+alien bot"], ["port_addbot humans", "+human bot"],
		["port_removebot aliens", "-alien bot"], ["port_removebot humans", "-human bot"], ["port_restart", "restart"],
		["port_maps", "maps"], ["port_help", "hide help"]]
	var parts := PackedStringArray()
	for r in rows:
		var k: String = game.keys_for(r[0])
		if k != "":
			parts.append("%s %s" % [k, r[1]])
	var out := ""
	for i in parts.size():
		out += parts[i] + ("\n" if i % 9 == 8 else "   ")
	return out + "\nEsc menu   console: bind <key> <command>, bindlist, callvote map <name>, vote yes, kill, ..."

var _shown_for_intermission := false
var _help_defaulted := false

var _team_menu_after_load := false
var _was_loading := false

func _process(_delta: float) -> void:
	# a map loading (CG_DrawLoadingScreen): menus, scores and texts go away
	# like the UI's on CL_MapLoading
	if game.is_loading():
		if not _was_loading:
			_was_loading = true
			close_menu()
			_show_scores(false)
			_shown_for_intermission = false
			lbl_center.text = ""
			lbl_vote.text = ""
			crosshair.visible = false
		return
	_was_loading = false
	if _team_menu_after_load:
		_team_menu_after_load = false
		open_menu("team")
	var h: Dictionary = game.get_hud()
	if h.is_empty():
		return
	var alive: bool = h.alive
	orig_hud = game.has_game_data()
	if orig_hud:
		# the original HUD (TremHud) draws all of this; scores and stages are
		# on the scoreboard (Tab), the team/class hints come from CG_TUTORIAL
		lbl_health.text = ""
		lbl_ammo.text = ""
	elif alive:
		lbl_health.text = "%d / %d  %s" % [h.health, h.max_health, h["class"]]
		var ammo := "" if h.ammo < 0 else "%d | %d  " % [h.ammo, h.clips]
		lbl_ammo.text = ammo + h.weapon
	elif h.spectating:
		lbl_health.text = h.team
		lbl_ammo.text = ""
	else:
		lbl_health.text = "DEAD"
		lbl_ammo.text = ""
	var funds := "%d %s" % [h.funds, "evos" if h.team == "Aliens" else "credits"]
	var bp := ""
	if h.blueprint != "" or h.build_timer > 0 or h.weapon_id in ["abuild", "abuildupg", "ckit"]:
		bp = "\nBP %d  build timer %.1fs  %s" % [h.bp, h.build_timer / 1000.0, ("[%s %s]" % [h.blueprint, "OK" if h.blueprint_valid else "blocked"]) if h.blueprint != "" else ""]
	if orig_hud:
		lbl_funds.text = ""
	else:
		lbl_funds.text = "%s   score %d\nAliens S%d (%d)   Humans S%d (%d)%s\n%s" % [funds, h.score, h.alien_stage, h.alien_credits, h.human_stage, h.human_credits, bp, h.items]
	# the original HUD shows the feed (CG_CONSOLE) and the location (CG_PLAYER_LOCATION)
	lbl_feed.text = "" if orig_hud else h.feed
	if h.get("location", "") != "" and not orig_hud:
		lbl_funds.text += "\n@ " + h.location
	var center := ""
	if h.winner != "" and h.get("intermission", false):
		center = h.winner + ("\nReady - waiting for the other players" if h.get("ready", false) else "\nFire to continue (next match starts when everyone is ready)")
	elif h.winner != "":
		center = h.winner
	elif h.get("center_print", "") != "" and not orig_hud:  # else CG_DrawCenterString
		center = h.center_print
	elif h.get("following", "") != "" and not orig_hud:  # else CG_FOLLOW
		center = "Following %s   (%s / %s cycle)" % [h.following, game.keys_for("weapprev"), game.keys_for("weapnext")]
	elif h.queued and not orig_hud:  # else CG_DrawQueue
		center = "Waiting to spawn (position %d)" % h.queue_pos
	elif orig_hud:
		pass
	elif h.spectating and h.team != "Spectators" and menu_kind == "":
		# the original menus open on fire (SpectatorThink's G_TriggerMenu)
		center = "Press fire to choose how to spawn" if trem_menus and trem_menus.has_menus() else "Press Esc to choose how to spawn"
	elif h.spectating and menu_kind == "":
		center = "Press fire to join a team" if trem_menus and trem_menus.has_menus() else "Press Esc to join a team"
	elif not alive and not h.spectating:
		center = "You died"
	lbl_center.text = center
	var vote_keys := "%s yes  %s no" % [game.keys_for("vote yes"), game.keys_for("vote no")]
	# the original HUD draws the votes (CG_DrawVote)
	lbl_vote.text = "" if orig_hud else h.get("vote", "") + (("\n" + vote_keys) if h.get("vote", "") != "" else "")
	var msg: String = h.get("message", "")
	if msg == "Choose a structure to build":
		msg = ""  # G_TriggerMenu( MN_x_BUILD ) prints nothing
	lbl_status.text = (h.status + "\n" + msg).strip_edges()
	crosshair.visible = alive and not orig_hud
	damage_flash.color.a = clamp(h.damage / 255.0, 0.0, 0.35) if alive else 0.0
	if h.wants_menu and menu_kind == "":
		# G_TriggerMenu: MN_A_BUILD / MN_H_BUILD (build_fire with nothing
		# selected), MN_A_INFEST (evolve) or the armoury
		if h.get("message", "") == "Choose a structure to build":
			open_menu("build")
		else:
			open_menu("evolve" if h.team == "Aliens" else "armoury")
	# the original HUD has its own context help (CG_TUTORIAL): ours starts hidden
	if orig_hud and not _help_defaulted:
		show_help = false
	_help_defaulted = true
	lbl_help.visible = show_help
	if lbl_help.visible and lbl_help.text == "":
		lbl_help.text = _help_text()
	if h.get("intermission", false):
		if not _shown_for_intermission:
			_show_scores(true)
		_shown_for_intermission = true
	elif _shown_for_intermission:
		_shown_for_intermission = false
		_show_scores(false)
	if score_panel.visible:
		var t := "%-16s %-8s %-14s %6s %5s %5s\n" % ["Name", "Team", "Class", "Score", "Kills", "Deaths"]
		for r in game.get_scores():
			t += "%-16s %-8s %-14s %6d %5d %5d%s\n" % [r.name, r.team, r["class"], r.score, r.kills, r.deaths, "  <" if r.me else ""]
		score_label.text = t
