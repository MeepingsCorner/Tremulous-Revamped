extends Node
## Screenshots of the cgame parts ported in this pass:
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/a_shot.tscn -- --part=1
## part 1 -> user://a_loading_1.png, a_loading_2.png (CG_DrawLoadingScreen while
##           ATCS loads), a_human.png (missing HUD images as the default image:
##           charge bar / team overlay), a_viewsize70.png (CG_CalcVrect +
##           CG_TileClear), a_follow.png ("following"), a_chase.png (chase cam,
##           "chasing"), a_wall.png (view weapon against a wall: RF_DEPTHHACK)
## part 2 -> user://a_water.png, a_water2.png (karith under water: fov_x / fov_y warp)
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
	print("a_shot: ", c, " -> ", "ok" if r == "" else r)
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
	print("a_shot: not alive after %.0f s" % limit)

func _shot(path: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	img.save_png(path)
	# (console() would finish a pending load at once)
	print("a_shot: saved ", ProjectSettings.globalize_path(path), " ", "loading" if game.is_loading() else game.console("cg_viewdebug"))

func _join(team: String, cls: String) -> void:
	_cmd("team " + team)
	_cmd("class " + cls)
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _look_at(classname: String, n: int, back: float) -> void:
	var v: PackedVector3Array = game.entity_view(classname, n)
	if v.size() == 2:
		var p := v[0]
		var a := v[1]
		var yaw := rad_to_deg(atan2(-(a.z - p.z), a.x - p.x))
		var d := Vector2(a.x - p.x, -(a.z - p.z)).normalized()
		_cmd("setviewpos %d %d %d %d" % [p.x * 32 - d.x * back, -p.z * 32 - d.y * back, p.y * 32 - 24, yaw])

func _run1() -> void:
	# the startup map loads across frames behind the loading screen
	var frames := 0
	var saved := 0
	while game.is_loading():
		frames += 1
		if (frames == 3 and saved == 0) or (frames == 12 and saved == 1):
			saved += 1
			await _shot("user://a_loading_%d.png" % saved)
		await get_tree().process_frame
	print("a_shot: loaded after %d frames" % frames)
	game.set_cheats(true)
	await _wait(0.3)
	_join("humans", "rifle")
	await _wait_alive(40.0)
	await _wait(1.0)
	await _shot("user://a_spawn.png")
	_look_at("reactor", 0, 0.0)
	await _wait(1.0)
	await _shot("user://a_human.png")
	game.exec("cg_viewsize 70\n")
	await _wait(0.6)
	await _shot("user://a_viewsize70.png")
	game.exec("cg_viewsize 100\n")
	# walk into a wall: the hand stays in front of it (RF_DEPTHHACK)
	var f: PackedStringArray = game.console("cg_viewdebug").split(" ")
	for yaw in [0, 90, 180, 270]:
		_cmd("setviewpos %s %s %d %d" % [f[1], f[2], int(float(f[3])) - 26, yaw])
		await _wait(0.2)
		var e := InputEventKey.new()
		e.physical_keycode = KEY_W
		e.keycode = KEY_W
		e.pressed = true
		Input.parse_input_event(e)
		await _wait(1.5)
		e = InputEventKey.new()
		e.physical_keycode = KEY_W
		e.keycode = KEY_W
		e.pressed = false
		Input.parse_input_event(e)
		await _wait(0.2)
		await _shot("user://a_wall_%d.png" % yaw)
	# spectate and follow a bot, then the chase cam (BUTTON_USE_HOLDABLE)
	_cmd("team spectate")
	await _wait(1.2)
	_cmd("follow")
	await _wait(1.0)
	await _shot("user://a_follow.png")
	game.exec("+button2\n")
	await _wait(0.5)
	game.exec("-button2\n")
	await _wait(1.0)
	await _shot("user://a_chase.png")
	get_tree().quit()

func _run2() -> void:
	print("a_shot: load karith -> '", game.load_bsp(game.pk3_paths(), "karith"), "'")
	game.set_cheats(true)
	await _wait(0.3)
	_join("humans", "rifle")
	await _wait_alive(40.0)
	# the pool of textures/karith/water: x -2528..-1792, y 5528..6912, z -968..-840
	_cmd("setviewpos -2160 6200 -950 90")
	await _wait(0.4)
	await _shot("user://a_water.png")
	await _wait(0.9)
	await _shot("user://a_water2.png")
	_cmd("setviewpos -2160 6200 -800 90")
	await _wait(0.6)
	await _shot("user://a_water_above.png")
	get_tree().quit()

func _run3() -> void:
	# serverinfo / CS_MOTD for the loading screen, then a map change
	print(game.console("set sv_hostname ^1Port^7 Test [smile] server"))
	print(game.console("set g_motd Welcome!\\nSecond line of the MOTD"))
	await _wait(0.5)
	print("a_shot: load_map tremor -> '", game.load_map(game.pk3_paths(), "tremor"), "'")
	var frames := 0
	var saved := 0
	while game.is_loading():
		frames += 1
		if (frames == 2 and saved == 0) or (frames == 20 and saved == 1):
			saved += 1
			await _shot("user://a_loading_tremor_%d.png" % saved)
		await get_tree().process_frame
	print("a_shot: tremor loaded after %d frames" % frames)
	await _wait(1.0)
	await _shot("user://a_tremor_after.png")
	get_tree().quit()

func _run4() -> void:
	# loading progress per frame (no console calls: they would finish the load)
	var t0 := Time.get_ticks_msec()
	while game.is_loading():
		print("a_shot: frame at %d ms loading" % (Time.get_ticks_msec() - t0))
		await get_tree().process_frame
	print("a_shot: loaded at %d ms" % (Time.get_ticks_msec() - t0))
	get_tree().quit()
