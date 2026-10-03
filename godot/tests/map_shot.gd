extends Node
## Loads a real map from the pk3 search path and saves screenshots:
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/map_shot.tscn -- --map=atcs
var main: Node
var game
var t := 0.0
var map := "atcs"
var step := 0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("map: data=", game.has_game_data(), " load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	game.command("team humans")
	game.command("class rifle")
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _shot(name: String) -> void:
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://" + name)
	print("map: saved ", name)

func _process(delta: float) -> void:
	t += delta
	if step == 0 and t > 14.0:
		step = 1
		var h: Dictionary = game.get_hud()
		print("map: alive=", h.alive, " team=", h.team)
		_shot("map_%s_fp.png" % map)
	elif step == 1 and t > 15.0:
		step = 2
		game.third_person = true
		_shot("map_%s_tp.png" % map)
	elif step == 2 and t > 16.0:
		get_tree().quit()
