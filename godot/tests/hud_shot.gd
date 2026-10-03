extends Node
## Screenshots of the original (menu-script) HUD drawn by TremHud:
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/hud_shot.tscn
## -> user://hud_human.png (+ hud_human_43.png at 4:3), hud_alien.png,
##    hud_builder.png (structure health bars), hud_spectator.png
var main: Node
var game

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.set_cheats(true)  # g_cheats: skips the 30 s team-change lock
	_run()

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	print("hud_shot: ", c, " -> ", "ok" if r == "" else r)
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
	print("hud_shot: not alive after %.0f s" % limit)

func _shot(path: String) -> void:
	# hide hud.gd's own labels so the original HUD can be inspected
	for c in main.get_node("HUD").get_children():
		if c is CanvasItem and c.name != "TremHud":
			c.visible = false
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	img.save_png(path)
	print("hud_shot: saved ", ProjectSettings.globalize_path(path))
	for c in main.get_node("HUD").get_children():
		if c is CanvasItem:
			c.visible = true

func _join(team: String, cls: String) -> void:
	_cmd("team " + team)
	_cmd("class " + cls)
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _run() -> void:
	await _wait(0.3)
	_join("humans", "rifle")
	await _wait_alive(30.0)
	# the scanner needs a helmet: stand by an armoury and buy one
	var v: PackedVector3Array = game.entity_view("arm", 0)
	print("hud_shot: armoury view ", v)
	if v.size() == 2:
		var p := v[0]
		var a := v[1]
		var yaw := rad_to_deg(atan2(-(a.z - p.z), a.x - p.x))
		_cmd("setviewpos %d %d %d %d" % [p.x * 32, -p.z * 32, p.y * 32 - 24, yaw])
		await _wait(0.3)
	if _cmd("buy helmet") != "":
		# no cheats (no setviewpos/give): show the scanner anyway for the shot
		main.get_node("HUD/TremHud").scanner_without_helmet = true
	# a chat line for the notify console (CG_CONSOLE)
	_cmd("say hello from the HUD test")
	await _wait(1.0)
	await _shot("user://hud_human.png")
	# 4:3 window: Menu_AspectCompensate with aspectScale 1
	var size := get_window().size
	get_window().size = Vector2i(960, 720)
	await _wait(0.5)
	await _shot("user://hud_human_43.png")
	get_window().size = size
	await _wait(0.3)
	main.get_node("HUD/TremHud").scanner_without_helmet = false
	_cmd("team spectate")
	await _wait(1.1)  # G_ChangeTeam ignores changes within 1 s
	_join("aliens", "level0")
	await _wait_alive(30.0)
	await _wait(1.0)
	await _shot("user://hud_alien.png")
	_cmd("team spectate")
	await _wait(1.1)  # G_ChangeTeam ignores changes within 1 s
	_join("aliens", "builder")
	await _wait_alive(30.0)
	# look at an egg from a few metres: the builder sees the structures' health bars
	var ov: PackedVector3Array = game.entity_view("eggpod", 1)
	print("hud_shot: eggpod view ", ov)
	if ov.size() == 2:
		var p := ov[0]
		var a := ov[1]
		var yaw := rad_to_deg(atan2(-(a.z - p.z), a.x - p.x))
		_cmd("setviewpos %d %d %d %d" % [p.x * 32, -p.z * 32, p.y * 32 - 24, yaw])
	await _wait(1.0)
	await _shot("user://hud_builder.png")
	_cmd("team spectate")
	await _wait(1.1)  # G_ChangeTeam ignores changes within 1 s
	await _wait(0.5)
	await _shot("user://hud_spectator.png")
	get_tree().quit()
