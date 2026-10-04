extends Node
## Enhanced dynamic shadows (gfx.rs shadow sun): a player throws a shadow
## on the floor with r_dynamicShadows 1, none with 0, none in vanilla; and
## the HD UI (hdtex.rs ui) in the enhanced mode.
## godot --path godot res://tests/shadow_shot.tscn -> "shadow: PASS|FAIL" lines, user://shadow_<n>.png
var main: Node
var game
var fails := []
var cam: Camera3D

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("shadow: load -> '", game.load_bsp(game.pk3_paths(), "atcs"), "'")
	main.get_node("HUD").close_menu()
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _check(name: String, ok: bool, detail: String) -> void:
	print("shadow: ", "PASS " if ok else "FAIL ", name, ": ", detail)
	if not ok:
		fails.append(name)

func _process(_d: float) -> void:
	if cam and not cam.current:
		cam.make_current()

func _shot(n: String) -> Image:
	main.get_node("HUD").close_menu()
	await get_tree().process_frame
	await get_tree().process_frame
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	img.save_png("user://shadow_%s.png" % n)
	return img

## mean luminance difference between two shots (sampled)
func _diff(a: Image, b: Image) -> float:
	var s := 0.0
	var n := 0
	for y in range(0, a.get_height(), 6):
		for x in range(0, a.get_width(), 6):
			s += abs(a.get_pixel(x, y).get_luminance() - b.get_pixel(x, y).get_luminance())
			n += 1
	return s / max(n, 1)

func _run() -> void:
	await _wait(3.0)
	game.set_cheats(true)
	game.exec("team humans")
	game.exec("class rifle")
	var t := 0.0
	while t < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t += 0.5
	# the HUD in vanilla and enhanced (the game's own view)
	game.exec("r_enhanced 0")
	await _wait(1.0)
	var hud_v := await _shot("hud_vanilla")
	game.exec("r_enhanced 1")
	await _wait(1.0)
	var hud_e := await _shot("hud_enhanced")
	_check("the HUD changes in the enhanced mode (HD art)", _diff(hud_v, hud_e) > 0.0005, "difference %.4f" % _diff(hud_v, hud_e))
	# the 3D view alone from here (no HUD text changing between shots)
	main.get_node("HUD").set("visible", false)
	# our body seen from outside, its shadow on the floor
	var me: int = game.my_entity()
	var at := Vector3.ZERO
	for l: String in game.actor_info():
		if l.begins_with("ent %d " % me):
			var a: int = l.find("pos=(")
			var b: int = l.find(")", a)
			var v: PackedStringArray = l.substr(a + 5, b - a - 5).split(",")
			at = Vector3(float(v[0]), float(v[1]), float(v[2]))
	cam = Camera3D.new()
	add_child(cam)
	cam.global_position = at + Vector3(2.2, 1.6, 1.2)
	cam.look_at(at + Vector3(0, -0.6, 0), Vector3.UP)
	cam.make_current()
	game.third_person = true
	game.exec("r_dynamicShadows 1")
	await _wait(1.5)
	var on := await _shot("on")
	game.exec("r_dynamicShadows 0")
	await _wait(1.0)
	var off := await _shot("off")
	var d := _diff(on, off)
	_check("our body throws a shadow (enhanced)", d > 0.002, "difference on/off %.4f" % d)
	game.exec("r_dynamicShadows 1")
	game.exec("r_enhanced 0")
	await _wait(1.0)
	var v1 := await _shot("vanilla")
	game.exec("r_dynamicShadows 0")
	var v0 := await _shot("vanilla_off")
	game.exec("r_dynamicShadows 1")
	var v2 := await _shot("vanilla_again")
	# (the scene moves a little between frames: the toggle must not add to it)
	_check("vanilla: no dynamic shadows", _diff(v1, v0) < _diff(v0, v2) + 0.002, "difference %.4f (frame to frame %.4f)" % [_diff(v1, v0), _diff(v0, v2)])
	game.exec("r_dynamicShadows 1")
	game.exec("r_enhanced 1")
	# first person: our own body's shadow still falls
	game.third_person = false
	cam.queue_free()
	cam = null
	await _wait(1.0)
	var fp := await _shot("first_person")
	# G: the human's flashlight (enhanced only)
	var before := await _shot("flashlight_off")
	game.exec("port_vision")
	var torch := await _shot("flashlight")
	await get_tree().process_frame
	var after := await _shot("flashlight_on2")
	var st: String = game.vision_state()
	_check("the flashlight is on and lights the view", st == "1 1 0" and _diff(before, torch) > _diff(torch, after) + 0.002, "state %s, difference %.4f (frame to frame %.4f)" % [st, _diff(before, torch), _diff(torch, after)])
	game.exec("r_enhanced 0")
	await _wait(1.0)
	st = game.vision_state()
	game.exec("port_vision")
	var st2: String = game.vision_state()
	_check("vanilla: no flashlight, G does nothing", st == "1 0 0" and st2 == "1 0 0", "state %s, after G %s" % [st, st2])
	game.exec("r_enhanced 1")
	game.exec("port_vision")
	await _wait(0.5)
	# an alien: night vision
	game.exec("team aliens")
	await _wait(1.0)
	game.exec("class level0")
	t = 0.0
	while t < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive and str(h.get("class_id", "")) != "":
			break
		await _wait(0.5)
		t += 0.5
	await _wait(1.5)
	var a0 := await _shot("alien")
	game.exec("port_vision")
	var a1 := await _shot("night_vision")
	st = game.vision_state()
	_check("the alien's night vision", st == "1 0 1" and _diff(a0, a1) > 0.02, "state %s, difference %.4f" % [st, _diff(a0, a1)])
	main.get_node("HUD").set("visible", true)
	print("SUMMARY shadow_shot status=%s fails=%d %s" % ["ok" if fails.is_empty() else "fail", fails.size(), ", ".join(fails)])
	get_tree().quit()
