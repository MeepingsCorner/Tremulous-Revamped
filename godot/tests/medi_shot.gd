extends Node
## The medistation's hologram (the red cross, a surface of medistat.md3)
## must rise over the station's own projector, not elsewhere.
## godot --path godot res://tests/medi_shot.tscn -> user://medi_<n>.png
var main: Node
var game
var cam: Camera3D

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("medi: load -> '", game.load_bsp(game.pk3_paths(), "atcs"), "'")
	main.get_node("HUD").close_menu()
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _process(_d: float) -> void:
	if cam and not cam.current:
		cam.make_current()

func _shot(n: String) -> void:
	main.get_node("HUD").close_menu()
	await get_tree().process_frame
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://medi_%s.png" % n)

func _run() -> void:
	await _wait(3.0)
	game.exec("r_enhanced 0")
	var at := Vector3.ZERO
	for cn in ["team_human_medistat", "medistat"]:
		var l: PackedVector3Array = game.find_entities(cn)
		if not l.is_empty():
			at = l[0]
			print("medi: ", cn, " at ", at)
	for l in game.actor_info():
		if l.contains("Medistat"):
			print("medi: actor ", l)
	cam = Camera3D.new()
	add_child(cam)
	cam.make_current()
	# from above, then from two sides
	cam.global_position = at + Vector3(0.01, 4.0, 0.0)
	cam.look_at(at, Vector3.FORWARD)
	await _wait(1.5)
	await _shot("top")
	cam.global_position = at + Vector3(3.0, 1.8, 0.0)
	cam.look_at(at + Vector3(0, 1.0, 0), Vector3.UP)
	await _wait(0.5)
	await _shot("east")
	cam.global_position = at + Vector3(0.0, 1.8, 3.0)
	cam.look_at(at + Vector3(0, 1.0, 0), Vector3.UP)
	await _wait(0.5)
	await _shot("south")
	game.exec("r_enhanced 1")
	await _wait(1.0)
	await _shot("south_enh")
	# atcs's floor grate (eq2_fgrate_01: alpha holes, no alphaFunc: solid)
	var g := Vector3(1152.0 / 32.0, 16.0 / 32.0, 0.0)
	cam.global_position = g + Vector3(2.0, 3.0, 1.0)
	cam.look_at(g, Vector3.UP)
	game.exec("r_enhanced 0")
	await _wait(1.0)
	await _shot("grate")
	game.exec("r_enhanced 1")
	await _wait(1.0)
	await _shot("grate_enh")
	print("SUMMARY medi_shot status=ok")
	get_tree().quit()
