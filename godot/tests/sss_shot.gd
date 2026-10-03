extends Node
## Subsurface scattering (models.rs sss_class): the player's own body in
## third person, facing the camera, in the vanilla and enhanced modes.
## godot --path godot res://tests/sss_shot.tscn -- --team=humans|aliens --class=rifle|level1
## -> user://sss_<team>_vanilla.png, user://sss_<team>_enhanced.png
var main: Node
var game
var team := "humans"
var klass := "rifle"

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--team="):
			team = a.substr(7)
		if a.begins_with("--class="):
			klass = a.substr(8)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("sss: load -> '", game.load_bsp(game.pk3_paths(), "atcs"), "'")
	main.get_node("HUD").close_menu()
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _shot(name: String) -> void:
	await _wait(1.5)
	get_viewport().get_texture().get_image().save_png("user://" + name)
	print("sss: saved ", name)

func _run() -> void:
	await _wait(3.0)
	game.exec("team " + team)
	game.exec("class " + klass)
	var t := 0.0
	while t < 20.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			break
		await _wait(0.5)
		t += 0.5
	print("sss: alive=", not game.get_hud().is_empty() and game.get_hud().alive, " as ", team, " ", klass)
	# third person, looking back at the face from close by
	game.exec("cg_thirdPerson 1")
	game.exec("cg_thirdPersonAngle 160")
	game.exec("cg_thirdPersonRange 55")
	game.exec("r_enhanced 0")
	await _shot("sss_%s_vanilla.png" % team)
	game.exec("r_enhanced 1")
	await _shot("sss_%s_enhanced.png" % team)
	# leave the archived settings as they were
	game.exec("cg_thirdPerson 0")
	game.exec("cg_thirdPersonAngle 0")
	game.exec("cg_thirdPersonRange 75")
	get_tree().quit()
