extends Node
## The building flow of the original (buildFire / G_TriggerMenu / build):
## a builder fires with nothing selected -> the team's build menu opens;
## `build <structure>` -> the ghost; fire -> the structure stands.
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/build_shot.tscn
## -> user://build_<team>_menu.png, build_<team>_ghost.png, build_<team>_built.png
var main: Node
var game
var menus

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.set_cheats(true)
	menus = main.get_node_or_null("HUD/TremMenus")
	_run()

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	print("build_shot: ", c, " -> ", "ok" if r == "" else r)
	return r

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(path: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png(path)
	print("build_shot: saved ", path)

func _fire() -> void:
	var m1: int = game.key_number("MOUSE1")
	game.key_event(m1, true)
	await _wait(0.4)
	game.key_event(m1, false)
	await _wait(0.4)

func _wait_alive(limit: float) -> void:
	var t := 0.0
	while t < limit:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			return
		await _wait(0.25)
		t += 0.25

func _stand_by(classname: String, turn: float) -> void:
	# stand at a free spot near the structure, turned `turn` degrees from it
	var v: PackedVector3Array = game.entity_view(classname, 0)
	if v.size() == 2:
		var p := v[0]
		var a := v[1]
		var yaw := rad_to_deg(atan2(-(a.z - p.z), a.x - p.x)) + turn
		_cmd("setviewpos %d %d %d %d" % [p.x * 32, -p.z * 32, p.y * 32 - 24, yaw])
	await _wait(0.5)

func _team(team: String, cls: String, near: String, build: String) -> bool:
	while game.is_loading():
		await _wait(0.5)
	_cmd("team spectate")
	await _wait(1.1)
	_cmd("team " + team)
	_cmd("class " + cls)
	main.get_node("HUD").close_menu()
	await _wait_alive(30.0)
	await _stand_by(near, 90.0)
	var n0: int = game.find_entities(build).size()
	# buildFire with nothing selected: G_TriggerMenu( MN_x_BUILD )
	await _fire()
	await _wait(0.5)
	var opened: String = menus.current_menu() if menus else ""
	print("build_shot: %s menu after fire: '%s'" % [team, opened])
	await _shot("user://build_%s_menu.png" % team)
	main.get_node("HUD").close_menu()
	await _wait(0.3)
	var n1 := n0
	for turn in [90.0, -90.0, 135.0, -135.0, 180.0, 45.0, -45.0]:
		main.get_node("HUD").close_menu()  # the error dialog of a failed try
		await _wait(0.3)
		await _stand_by(near, turn)
		_cmd("build " + build)
		await _wait(0.6)
		await _shot("user://build_%s_ghost.png" % team)
		await _fire()
		await _wait(1.5)
		n1 = game.find_entities(build).size()
		if n1 > n0:
			break
		print("build_shot: turn %d: %s" % [turn, str(game.get_hud().get("message", ""))])
	await _shot("user://build_%s_built.png" % team)
	var ok := opened.ends_with("build") and n1 > n0
	print("build_shot: %s %s %d -> %d (%s)" % [team, build, n0, n1, "PASS" if ok else "FAIL: " + str(game.get_hud().get("message", ""))])
	return ok

func _run() -> void:
	await _wait(0.5)
	var ok_h := await _team("humans", "ckit", "reactor", "medistat")
	var ok_a := await _team("aliens", "builder", "overmind", "eggpod")
	print("build_shot: ", "PASS" if ok_h and ok_a else "FAIL")
	get_tree().quit()
