extends Node
## A bot match on a real map with the local player following bots:
## screenshots every few seconds to eyeball effects/HUD in play.
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/play_shot.tscn -- --map=atcs
var main: Node
var game
var t := 0.0
var map := "atcs"
var shots := 0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--map="):
			map = a.substr(6)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("play: load ", map, " -> '", game.load_bsp(game.pk3_paths(), map), "'")
	main.get_node("HUD").close_menu()
	if OS.get_environment("TREM_NO_HUD") != "":
		main.get_node("HUD/TremHud").queue_free()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _process(delta: float) -> void:
	t += delta
	if t > 2.0 and shots == 0:
		shots = 1
		if OS.get_environment("TREM_NO_FOLLOW") == "":
			print("play: follow -> ", game.command("follow"))
	if t > 6.0 + shots * 5.0 and shots <= 6:
		await RenderingServer.frame_post_draw
		get_viewport().get_texture().get_image().save_png("user://play_%s_%d.png" % [map, shots])
		print("play: saved ", shots, " ", game.get_hud().get("following", ""))
		shots += 1
		if shots % 2 == 0:
			game.command("follownext")
	if shots > 6:
		get_tree().quit()
