extends Node
## Screenshots of the graphics modes (gfx.rs / rt.rs) from the same views:
## godot --path godot res://tests/gfx_shot.tscn -- --map=atcs
## -> user://gfx_<map>_<view>_<mode>.png for the modes vanilla, enhanced
##    (no ray tracing), rt (ray tracing), pt (path tracing), rt_sw / pt_sw
##    (the software tracer), and
##    gfx_<map>_<view>_dbg<n>.png: the ray tracer's AO / indirect / reflections
var main: Node
var game
var map := "atcs"

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("gfx: load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(name: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://" + name)
	print("gfx: saved ", name)

func _hold(v: Vector4, secs: float) -> void:
	# keep the view still while the ray tracer accumulates
	var t := 0.0
	while t < secs:
		game.command("setviewpos %d %d %d %d" % [v.x, v.y, v.z, v.w])
		await _wait(0.1)
		t += 0.1

func _run() -> void:
	await _wait(3.0)
	game.set_cheats(true)
	game.command("team humans")
	game.command("class rifle")
	var t := 0.0
	while t < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t += 0.5
	await _wait(1.0)
	main.get_node("HUD").close_menu()
	if main.has_node("HUD/TremHud"):
		main.get_node("HUD/TremHud").queue_free()
	game.command("god")
	print("gfx: info ", game.gfx("info"))
	var p: Vector4 = game.view_pos()
	print("gfx: view ", p)
	# views looking at the main structures (entity_view: a free spot near them)
	var views := []
	for cls in ["team_human_reactor", "team_alien_overmind", "team_human_spawn"]:
		var ev: PackedVector3Array = game.entity_view(cls, 0)
		if ev.size() == 2:
			var eye := Vector3(ev[0].x * 32.0, -ev[0].z * 32.0, ev[0].y * 32.0 - 20.0)
			var at := Vector3(ev[1].x * 32.0, -ev[1].z * 32.0, ev[1].y * 32.0)
			views.append(Vector4(eye.x, eye.y, eye.z, rad_to_deg(atan2(at.y - eye.y, at.x - eye.x))))
	if views.is_empty():
		views = [Vector4(p.x, p.y, p.z, p.w + 180.0)]
	var i := 0
	for v in views:
		for mode in ["vanilla", "enhanced", "rt", "pt", "rt_sw", "pt_sw"]:
			game.gfx("hardware")
			if mode == "enhanced":
				game.gfx("enhanced")
				game.gfx("off")
			elif mode.ends_with("_sw"):
				game.gfx("software")
				game.gfx(mode.substr(0, 2))
			else:
				game.gfx(mode)
			await _hold(v, 1.6)
			await _shot("gfx_%s_%d_%s.png" % [map, i, mode])
		if i == 0:
			for dbg in [1, 2, 3]:
				game.gfx("debug %d" % dbg)
				await _hold(v, 1.0)
				await _shot("gfx_%s_%d_dbg%d.png" % [map, i, dbg])
			game.gfx("debug 0")
		i += 1
	game.gfx("hardware")
	game.gfx("auto")
	print("gfx: info ", game.gfx("info"))
	get_tree().quit()
