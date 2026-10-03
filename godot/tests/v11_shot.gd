extends Node
## The Godot client on a vanilla Tremulous 1.1.0 server (dec040, dec041):
## connects (Auto picks protocol 69), joins the humans with a rifle, walks,
## shoots, then joins the aliens as a granger and builds; saves what it sees
## (user://v11_<n>.png).
## xvfb-run -a godot --rendering-driver opengl3 --path godot res://tests/v11_shot.tscn -- --port=30780
var main: Node
var game
var port := 30780
var t := 0.0
var step := 0
var shots := 0
var last_log := 0.0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--port="):
			port = int(a.substr(7))
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	print("v11_shot: ", game.exec("set name GodotV11\nconnect 127.0.0.1:%d" % port))

func shot(tag: String) -> void:
	get_viewport().get_texture().get_image().save_png("user://v11_%d.png" % shots)
	print("v11_shot: saved %d (%s) %s" % [shots, tag, game.net_status()])
	shots += 1

func _process(delta: float) -> void:
	t += delta
	if t - last_log > 3.0:
		last_log = t
		print("v11_shot t=%.0f step=%d %s" % [t, step, game.net_status()])
	match step:
		0:
			if t > 6.0:
				print("v11_shot: ", game.exec("team humans"))
				step = 1
		1:
			if t > 8.0:
				print("v11_shot: ", game.exec("class rifle"))
				step = 2
		2:
			if t > 12.0:
				shot("human spawned")
				game.exec("+forward")
				step = 3
		3:
			if t > 14.0:
				game.exec("-forward\n+attack")
				step = 4
		4:
			if t > 15.5:
				game.exec("-attack")
				shot("human fired")
				print("v11_shot: ", game.exec("team aliens"))
				step = 5
		5:
			if t > 18.0:
				print("v11_shot: ", game.exec("class builder"))
				step = 6
		6:
			if t > 22.0:
				shot("granger spawned")
				print("v11_shot: ", game.exec("build eggpod"))
				step = 7
		7:
			if t > 23.0:
				game.exec("+attack")
				step = 8
		8:
			if t > 23.3:
				game.exec("-attack")
				step = 9
		9:
			if t > 27.0:
				shot("granger built")
				print("v11_shot: done ", game.net_status())
				get_tree().quit()
