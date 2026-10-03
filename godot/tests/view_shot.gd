extends Node
## Screenshots of the cgame view (view.rs: CG_CalcViewValues, CG_AddViewWeapon,
## CG_GhostBuildable):
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/view_shot.tscn -- --part=1
## part 1 -> user://view_human.png (hand + rifle, class fov), view_third.png
##           (third person), view_zoom.png (mass driver zoomed),
##           view_painblend.png (fall damage), view_ghost.png
## part 2 -> user://view_dretch.png (dretch fov, no view weapon in the data),
##           view_ghost_alien.png
## part 3 -> user://view_ghost_{0,90,180}.png (the ghost from three sides)
## part 4 -> user://view_wallwalk.png, view_wallwalk2.png (basilisk on a wall)
var main: Node
var game
var part := "1"

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--part="):
			part = a.substr(7)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.set_cheats(true)
	if part == "1":
		_run1()
	elif part == "3":
		_run3()
	elif part == "4":
		_run4()
	else:
		_run2()

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	print("view_shot: ", c, " -> ", "ok" if r == "" else r)
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
	print("view_shot: not alive after %.0f s" % limit)

func _shot(path: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	img.save_png(path)
	print("view_shot: saved ", ProjectSettings.globalize_path(path), " ", game.console("cg_viewdebug"))

func _join(team: String, cls: String) -> void:
	_cmd("team " + team)
	_cmd("class " + cls)
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _respawn(team: String, cls: String) -> void:
	_cmd("team spectate")
	await _wait(1.1)  # G_ChangeTeam ignores changes within 1 s
	_join(team, cls)
	await _wait_alive(40.0)
	await _wait(0.5)

func _key(k: Key, down: bool) -> void:
	var e := InputEventKey.new()
	e.physical_keycode = k
	e.keycode = k
	e.pressed = down
	Input.parse_input_event(e)

func _look_at(classname: String, n: int, back: float) -> void:
	var v: PackedVector3Array = game.entity_view(classname, n)
	print("view_shot: ", classname, " view ", v)
	if v.size() == 2:
		var p := v[0]
		var a := v[1]
		var yaw := rad_to_deg(atan2(-(a.z - p.z), a.x - p.x))
		var d := Vector2(a.x - p.x, -(a.z - p.z)).normalized()
		_cmd("setviewpos %d %d %d %d" % [p.x * 32 - d.x * back, -p.z * 32 - d.y * back, p.y * 32 - 24, yaw])

func _run1() -> void:
	await _wait(0.3)
	_join("humans", "rifle")
	await _wait_alive(40.0)
	_look_at("arm", 0, 0.0)
	await _wait(1.0)
	await _shot("user://view_human.png")
	game.third_person = true
	await _wait(0.6)
	await _shot("user://view_third.png")
	game.third_person = false
	# mass driver, zoomed with the secondary attack held
	_cmd("give funds")
	_cmd("sell rifle")
	_cmd("buy mdriver")
	await _wait(0.6)
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
	var mb := InputEventMouseButton.new()
	mb.button_index = MOUSE_BUTTON_RIGHT
	mb.pressed = true
	Input.parse_input_event(mb)
	await _wait(1.0)
	await _shot("user://view_zoom.png")
	mb = InputEventMouseButton.new()
	mb.button_index = MOUSE_BUTTON_RIGHT
	mb.pressed = false
	Input.parse_input_event(mb)
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	# a long fall: landing kick, fall damage, CG_PainBlend
	var f: PackedStringArray = game.console("cg_viewdebug").split(" ")
	_cmd("setviewpos %s %s %d 180" % [f[1], f[2], int(float(f[3])) + 260])
	for i in 12:
		await _wait(0.25)
		var h: Dictionary = game.get_hud()
		print("view_shot: falling health ", h.get("health", "?"))
		if int(h.get("health", 100)) < 100:
			break
	await _wait(0.2)
	await _shot("user://view_painblend.png")
	await _respawn("humans", "ckit")
	_look_at("arm", 0, 150.0)
	await _wait(0.3)
	_cmd("build mgturret")
	await _wait(1.0)
	await _shot("user://view_ghost.png")
	get_tree().quit()

func _run2() -> void:
	await _wait(0.3)
	_join("aliens", "level0")
	await _wait_alive(40.0)
	_look_at("eggpod", 0, 0.0)
	await _wait(1.0)
	await _shot("user://view_dretch.png")
	await _respawn("aliens", "builder")
	_look_at("eggpod", 1, 60.0)
	await _wait(0.3)
	_cmd("build acid_tube")
	await _wait(1.0)
	await _shot("user://view_ghost_alien.png")
	get_tree().quit()

func _run3() -> void:
	await _wait(0.3)
	_join("humans", "ckit")
	await _wait_alive(40.0)
	_look_at("arm", 0, 150.0)
	await _wait(0.3)
	_cmd("build mgturret")
	for yaw in [0, 90, -90]:
		var v: PackedVector3Array = game.entity_view("arm", 0)
		var p := v[0]
		_cmd("setviewpos %d %d %d %d" % [p.x * 32 + 150, -p.z * 32, p.y * 32 - 24, yaw])
		await _wait(0.8)
		await _shot("user://view_ghost_%d.png" % (yaw + 90))
	get_tree().quit()

func _run4() -> void:
	await _wait(0.3)
	_join("aliens", "level0")
	await _wait_alive(40.0)
	# evolve to a basilisk (cheat funds)
	_cmd("give funds")
	_cmd("class level1")
	await _wait(3.0)
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	game.input_enabled = true
	print("view_shot: hud ", game.get_hud().get("class", "?"))
	# hold crouch (wall walk) and walk ahead until on a wall, turning 90
	# degrees every few seconds
	_key(KEY_C, true)
	_key(KEY_W, true)
	var yaw := 0
	var climbing := 0
	for i in 60:
		await _wait(0.5)
		var d: String = game.console("cg_viewdebug")
		print("view_shot: ", d)
		if d.find("lastNormal 0.00 0.00 1.00") < 0:
			climbing += 1
			if climbing == 2:
				await _shot("user://view_wallwalk.png")
			if climbing == 4:
				await _shot("user://view_wallwalk2.png")
				break
		elif i % 8 == 7:
			var f := d.split(" ")
			yaw += 90
			_cmd("setviewpos %s %s %d %d" % [f[1], f[2], int(float(f[3])) - 1, yaw])
	_key(KEY_W, false)
	_key(KEY_C, false)
	get_tree().quit()
