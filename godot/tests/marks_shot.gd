extends Node
## Wall marks as real meshes (marks.rs: CG_ImpactMark + R_MarkFragments,
## CG_PlayerShadow, CG_Creep), which the Compatibility renderer draws too:
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/marks_shot.tscn
## -> user://c_marks_bullets.png (rifle bullet marks on the floor and wall),
##    user://c_marks_shadow.png (the player's blob shadow, third person),
##    user://c_marks_creep.png (creep under an egg)
var main: Node
var game

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.set_cheats(true)
	_run()

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	print("marks_shot: ", c, " -> ", "ok" if r == "" else r)
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
	print("marks_shot: not alive after %.0f s" % limit)

func _shot(path: String) -> void:
	var mk = game.get_node_or_null("Marks")
	if mk and mk.get_child_count() > 0:
		var mi: MeshInstance3D = mk.get_child(0)
		var m: Mesh = mi.mesh
		print("marks_shot: mesh surfaces ", m.get_surface_count(), " aabb ", m.get_aabb(), " visible ", mi.is_visible_in_tree())
		for si in m.get_surface_count():
			var arr = m.surface_get_arrays(si)
			print("marks_shot:  surface ", si, " verts ", arr[Mesh.ARRAY_VERTEX].size(), " mat ", m.surface_get_material(si))
	else:
		print("marks_shot: no Marks node")
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png(path)
	print("marks_shot: saved ", ProjectSettings.globalize_path(path), " ", game.console("cg_viewdebug"),)

func _join(team: String, cls: String) -> void:
	_cmd("team " + team)
	_cmd("class " + cls)
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _respawn(team: String, cls: String) -> void:
	_cmd("team spectate")
	await _wait(1.1)
	_join(team, cls)
	await _wait_alive(40.0)
	await _wait(0.5)

func _look_at(classname: String, n: int, back: float) -> void:
	var v: PackedVector3Array = game.entity_view(classname, n)
	if v.size() == 2:
		var p := v[0]
		var a := v[1]
		var yaw := rad_to_deg(atan2(-(a.z - p.z), a.x - p.x))
		var d := Vector2(a.x - p.x, -(a.z - p.z)).normalized()
		_cmd("setviewpos %d %d %d %d" % [p.x * 32 - d.x * back, -p.z * 32 - d.y * back, p.y * 32 - 24, yaw])

func _pitch(down: float) -> void:
	# mouse look down until the view pitch reaches `down` degrees
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
	game.input_enabled = true
	for i in 40:
		var f: PackedStringArray = game.console("cg_viewdebug").split(" ")
		if f.size() > 5 and float(f[5]) >= down:
			break
		var m := InputEventMouseMotion.new()
		m.relative = Vector2(0, 6)
		Input.parse_input_event(m)
		await _wait(0.05)

func _fire(secs: float) -> void:
	var mb := InputEventMouseButton.new()
	mb.button_index = MOUSE_BUTTON_LEFT
	mb.pressed = true
	Input.parse_input_event(mb)
	await _wait(secs)
	mb = InputEventMouseButton.new()
	mb.button_index = MOUSE_BUTTON_LEFT
	mb.pressed = false
	Input.parse_input_event(mb)

func _run() -> void:
	await _wait(0.3)
	for i in 3:
		game.remove_bot("aliens")
		game.remove_bot("humans")
	_join("humans", "rifle")
	await _wait_alive(40.0)
	_look_at("arm", 0, 120.0)
	await _wait(0.5)
	await _pitch(35.0)
	game.third_person = true
	await _wait(0.8)
	await _shot("user://c_marks_shadow.png")
	game.third_person = false
	# rifle fire into the lit floor ahead
	await _pitch(60.0)
	await _fire(1.5)
	await _pitch(40.0)
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	await _wait(0.3)
	await _shot("user://c_marks_bullets.png")
	await _respawn("aliens", "level0")
	_look_at("eggpod", 0, 0.0)
	await _wait(0.4)
	await _pitch(20.0)
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	await _wait(0.6)
	await _shot("user://c_marks_creep.png")
	get_tree().quit()
