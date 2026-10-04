extends Node
## Map surfaces drawn stage by stage (q3shader.rs): screenshots of the
## surfaces whose shaders use several stages, waves, tcMods, deforms, skies
## and fog, from the views in res://tests/stage_views.json (or $TREM_VIEWS):
## godot --path godot res://tests/stage_shot.tscn -- --map=atcs [--enhanced]
## [--burst=N] -> user://stage_<map>_<n>_<surface>_<mode>[_t|_b<k>].png ("_t": 0.7 s
## later, "_b<k>": N consecutive frames after it)
var main: Node
var game
var map := "atcs"
var enhanced := false
var burst := 0
var cam_xform = null

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
		if a == "--enhanced":
			enhanced = true
		if a.begins_with("--burst="):
			burst = int(a.substr(8))
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("stage: load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	RenderingServer.frame_pre_draw.connect(_pre_draw)
	_run()

func _pre_draw() -> void:
	if cam_xform == null:
		return
	var cam = game.view_camera()
	if cam:
		cam.global_transform = cam_xform
		RenderingServer.camera_set_transform(cam.get_camera_rid(), cam_xform)

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(name: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://" + name)
	print("stage: saved ", name)

func _g(q: Array) -> Vector3:
	return Vector3(q[0], q[2], -q[1]) / 32.0

func _run() -> void:
	await _wait(3.0)
	game.set_cheats(true)
	game.command("team spectate")
	game.console("cg_drawGun 0")
	await _wait(1.0)
	main.get_node("HUD").close_menu()
	if main.has_node("HUD/TremHud"):
		main.get_node("HUD/TremHud").visible = false
	var views: Dictionary = JSON.parse_string(FileAccess.get_file_as_string("res://tests/" + (OS.get_environment("TREM_VIEWS") if OS.get_environment("TREM_VIEWS") != "" else "stage_views.json")))
	var modes := ["vanilla"]
	if enhanced:
		modes.append("enhanced")
	var i := 0
	for v in views.get(map, []):
		var eye := _g(v.eye)
		var at := _g(v.at)
		cam_xform = Transform3D(Basis(), eye).looking_at(at, Vector3.UP if abs((at - eye).normalized().y) < 0.99 else Vector3.FORWARD)
		for mode in modes:
			game.gfx(mode if mode == "enhanced" else "vanilla")
			await _wait(1.0)
			await _shot("stage_%s_%d_%s_%s.png" % [map, i, v.name, mode])
			await _wait(0.7)
			await _shot("stage_%s_%d_%s_%s_t.png" % [map, i, v.name, mode])
			# consecutive frames: flicker shows as pixels that change frame to frame
			for b in burst:
				await _shot("stage_%s_%d_%s_%s_b%d.png" % [map, i, v.name, mode, b])
		i += 1
	print("stage: SUMMARY done ", map)
	get_tree().quit()
