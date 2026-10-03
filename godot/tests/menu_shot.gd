extends Node
## Screenshots of the original (legacy) menus drawn by TremMenus:
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/menu_shot.tscn
## -> user://menu_<kind>.png (team, team_humans_selected, spawn_human,
##    armoury, build_human, dialog, spawn_alien, evolve, build_alien,
##    ingame, ingame_game (map call vote), callvote_players, ignore,
##    help, say, scores)
## Clicks are given in 640x480 virtual coordinates of the 16:9 window, i.e.
## after the menus' aspect compensation (aspectScale 0.75).
var main: Node
var game
var menus
var hud

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.set_cheats(true)  # g_cheats: skips the 30 s team-change lock
	hud = main.get_node("HUD")
	menus = main.get_node("HUD/TremMenus")
	_run()

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	print("menu_shot: ", c, " -> ", "ok" if r == "" else r)
	return r

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _wait_alive(limit: float) -> void:
	var t := 0.0
	while t < limit:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			return
		await _wait(0.25)
		t += 0.25
	print("menu_shot: not alive after %.0f s" % limit)

func _shot(kind: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	var path := "user://menu_%s.png" % kind
	img.save_png(path)
	print("menu_shot: %s open=%s menu=%s -> %s" % [kind, menus.is_open(), menus.current_menu(), ProjectSettings.globalize_path(path)])

## a click at a 640x480 virtual position
func _click(vx: float, vy: float, double := false) -> void:
	var size := get_viewport().get_visible_rect().size
	var pos := Vector2(vx * size.x / 640.0, vy * size.y / 480.0)
	var mv := InputEventMouseMotion.new()
	mv.position = pos
	mv.global_position = pos
	Input.parse_input_event(mv)
	await get_tree().process_frame
	for i in (2 if double else 1):
		for pressed in [true, false]:
			var ev := InputEventMouseButton.new()
			ev.button_index = MOUSE_BUTTON_LEFT
			ev.pressed = pressed
			ev.position = pos
			ev.global_position = pos
			Input.parse_input_event(ev)
			await get_tree().process_frame

func _key(code: Key, unicode := 0) -> void:
	var ev := InputEventKey.new()
	ev.keycode = code
	ev.physical_keycode = code
	ev.unicode = unicode
	ev.pressed = true
	Input.parse_input_event(ev)
	await get_tree().process_frame
	var up := ev.duplicate()
	up.pressed = false
	Input.parse_input_event(up)
	await get_tree().process_frame

func _type(text: String) -> void:
	for c in text:
		var code: Key = KEY_SPACE if c == " " else OS.find_keycode_from_string(c.to_upper())
		await _key(code, c.unicode_at(0))

func _open(kind: String) -> void:
	hud.close_menu()
	hud.open_menu(kind)
	await _wait(0.3)

func _run() -> void:
	await _wait(0.5)
	# hud.gd opens the team menu at start
	print("menu_shot: start menu=", menus.current_menu(), " hud kind=", hud.menu_kind)
	await _shot("team")
	# pick "Humans" (row 2) and press OK: JoinTeam runs "cmd team humans"
	await _click(180, 156)
	await _wait(0.2)
	await _shot("team_humans_selected")
	await _click(423, 342)
	await _wait(0.3)
	print("menu_shot: after OK team=", game.get_hud().team, " menu=", menus.current_menu())
	if game.get_hud().team != "Humans":
		_cmd("team humans")
	# legacy: the spawn menu comes from G_TriggerMenu( MN_H_SPAWN ) on fire
	await _open("spawn")
	await _shot("spawn_human")
	hud.close_menu()
	_cmd("class rifle")
	await _wait_alive(30.0)
	await _open("armoury")
	await _shot("armoury")
	hud.close_menu()
	_cmd("team spectate")
	await _wait(1.1)  # G_ChangeTeam ignores changes within 1 s
	_cmd("team humans")
	_cmd("class ckit")
	await _wait_alive(30.0)
	await _open("build")
	await _shot("build_human")
	# a refusal dialog (cgame CG_Menu longMsg)
	hud.close_menu()
	menus.show_dialog("Insufficient funds. You do not have enough credits to perform this action.")
	await _wait(0.3)
	print("menu_shot: hud kind after dialog=", hud.menu_kind)
	await _shot("dialog")
	# Esc closes it (UI_KeyEvent: Menus_CloseAll; TremMenus signals hud.gd)
	await _key(KEY_ESCAPE)
	print("menu_shot: after Esc open=%s hud kind='%s' input_enabled=%s" % [menus.is_open(), hud.menu_kind, game.input_enabled])
	# the in-game Esc menu: Esc with no menu open (UI_SetActiveMenu( UIMENU_INGAME ))
	await _key(KEY_ESCAPE)
	await _wait(0.2)
	print("menu_shot: Esc -> menu=%s hud kind='%s'" % [menus.current_menu(), hud.menu_kind])
	await _shot("ingame")
	# "Game": ingame_game opens on its Vote/Map tab (callvote map ...)
	await _click(42, 28)
	await _wait(0.3)
	await _shot("ingame_game")
	# the "Players" vote tab
	await _click(26, 147)
	await _wait(0.2)
	await _shot("callvote_players")
	# the Ignore tab (FEEDER_IGNORE_LIST)
	await _click(150, 85)
	await _wait(0.2)
	await _shot("ignore")
	# "Help" on the bar: ingame_help (FEEDER_HELP_LIST + UI_HELPINFOPANE)
	await _click(136, 28)
	await _wait(0.2)
	# second topic
	await _click(60, 101)
	await _wait(0.2)
	await _shot("help")
	# Options -> Controls -> Move: the ITEM_TYPE_BIND items show the client's
	# bind table (Item_Bind_Paint); Item_Bind_HandleKey rebinds Sprint
	var had_cfg := FileAccess.file_exists("user://autogen.cfg")
	var saved_cfg := FileAccess.get_file_as_string("user://autogen.cfg") if had_cfg else ""
	for step in [["ingame", "options"], ["ingame_options", "controlsBtn"], ["ingame_options", "Move"]]:
		var c: Vector2 = menus.item_center(step[0], step[1])
		print("menu_shot: click %s/%s at %s" % [step[0], step[1], c])
		await _click(c.x, c.y)
		await _wait(0.3)
	await _shot("controls")
	var sc: Vector2 = menus.item_center("ingame_options", "+button8")
	await _click(sc.x, sc.y)
	await _wait(0.2)
	print("menu_shot: bind pending=%s" % menus.key_bind_pending())
	await _shot("bind_wait")
	await _key(KEY_K, "k".unicode_at(0))
	await _wait(0.3)
	print("menu_shot: after K pending=%s sprint keys=%s" % [menus.key_bind_pending(), game.keys_for("+button8")])
	await _shot("bind_done")
	# put the binds and autogen.cfg back
	game.exec("unbind k")
	await _wait(0.3)
	if had_cfg:
		var f := FileAccess.open("user://autogen.cfg", FileAccess.WRITE)
		f.store_string(saved_cfg)
		f.close()
	else:
		DirAccess.remove_absolute(ProjectSettings.globalize_path("user://autogen.cfg"))
	await _key(KEY_ESCAPE)
	print("menu_shot: after Esc open=%s hud kind='%s'" % [menus.is_open(), hud.menu_kind])
	# messagemode: T opens the say menu, typing fills ui_sayBuffer
	await _key(KEY_T, "t".unicode_at(0))
	await _type("hello aliens")
	await _shot("say")
	await _key(KEY_ENTER)
	await _wait(0.2)
	print("menu_shot: after say open=%s feed=%s" % [menus.is_open(), str(game.get_hud().feed).replace("\n", " | ")])
	hud.close_menu()
	_cmd("team spectate")
	await _wait(1.1)  # G_ChangeTeam ignores changes within 1 s
	_cmd("team aliens")
	await _open("spawn")
	await _shot("spawn_alien")
	hud.close_menu()
	_cmd("class level0")
	await _wait_alive(30.0)
	menus.test_credits = 20000
	await _open("evolve")
	await _shot("evolve")
	menus.test_credits = -1
	hud.close_menu()
	_cmd("team spectate")
	await _wait(1.1)  # G_ChangeTeam ignores changes within 1 s
	_cmd("team aliens")
	_cmd("class builder")
	await _wait_alive(30.0)
	await _open("build")
	await _shot("build_alien")
	hud.close_menu()
	await _wait(2.0)
	hud._show_scores(true)
	await _wait(0.3)
	await _shot("scores")
	hud._show_scores(false)
	get_tree().quit()
