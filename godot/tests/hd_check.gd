extends Node
## The 4K pack in the enhanced mode (dec055): screenshots of the main
## structures' views, to look at after changing the pack's files:
## godot --path godot res://tests/hd_check.tscn -- --map=atcs
## -> user://hdcheck_<map>_<view>.png
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
	print("hd: load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

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
	game.gfx("enhanced")
	game.gfx("off")
	var views := []
	for cls in ["team_human_reactor", "team_alien_overmind", "team_human_spawn"]:
		var ev: PackedVector3Array = game.entity_view(cls, 0)
		if ev.size() == 2:
			var eye := Vector3(ev[0].x * 32.0, -ev[0].z * 32.0, ev[0].y * 32.0 - 20.0)
			var at := Vector3(ev[1].x * 32.0, -ev[1].z * 32.0, ev[1].y * 32.0)
			views.append(Vector4(eye.x, eye.y, eye.z, rad_to_deg(atan2(at.y - eye.y, at.x - eye.x))))
	# where the player stands, looking both ways
	var p: Vector4 = game.view_pos()
	views.append(p)
	views.append(Vector4(p.x, p.y, p.z, p.w + 180.0))
	var i := 0
	for v in views:
		for k in 15:
			game.command("setviewpos %d %d %d %d" % [v.x, v.y, v.z, v.w])
			await _wait(0.1)
		await RenderingServer.frame_post_draw
		await RenderingServer.frame_post_draw
		var name := "hdcheck_%s_%d.png" % [map, i]
		get_viewport().get_texture().get_image().save_png("user://" + name)
		print("hd: saved ", name)
		i += 1
	print("hd: info ", game.gfx("info"))
	get_tree().quit()
