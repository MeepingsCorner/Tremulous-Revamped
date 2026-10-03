extends Node
## Map entities on a real map: looks at the first entity of each listed
## classname from a fixed offset and saves a screenshot.
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/ents_shot.tscn -- --map=nexus6 --class=func_door
var main: Node
var game
var t := 0.0
var map := "nexus6"
var classes := ["func_door"]
var step := 0
var cam: Camera3D
var targets := []

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
		if a.begins_with("--class="):
			classes = a.substr(8).split(",")
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("ents: load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	cam = Camera3D.new()
	add_child(cam)

func _process(delta: float) -> void:
	t += delta
	if step == 0 and t > 2.0:
		step = 1
		for c in classes:
			var p: PackedVector3Array = game.find_entities(c)
			print("ents: ", c, " x", p.size())
			if p.size() > 0:
				targets.append([c, game.entity_view(c, 0)])
	elif step >= 1 and step <= targets.size() and t > 2.0 + step * 1.5:
		var tgt = targets[step - 1]
		var v: PackedVector3Array = tgt[1]
		cam.look_at_from_position(v[0], v[1])
		cam.current = true
		await RenderingServer.frame_post_draw
		get_viewport().get_texture().get_image().save_png("user://ents_%s_%s.png" % [map, tgt[0]])
		print("ents: saved ", tgt[0])
		step += 1
	elif step > targets.size() and step > 0 and t > 4.0 + step * 1.5:
		get_tree().quit()
