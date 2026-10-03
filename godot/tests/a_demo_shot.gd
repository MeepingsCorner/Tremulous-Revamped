extends Node
## wt-a (dec037): a single-player game is recorded (`record a_demo`: the map
## restarts on the local server's loopback, as the original always plays),
## the player walks, `stoprecord`, then `demo a_demo` plays it back through
## the client and cgame; screenshots user://a_demo_rec.png and
## user://a_demo_play_<n>.png, then the demo ends (CL_DemoCompleted).
## xvfb-run -a godot --rendering-driver opengl3 --path godot res://tests/a_demo_shot.tscn
var main: Node
var game
var t := 0.0
var phase := 0
var phase_t := 0.0
var last_log := 0.0
var shots := 0

func _ready() -> void:
	DirAccess.remove_absolute("user://demos/a_demo.dm_71")
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 1
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func shot(name: String) -> void:
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png("user://%s.png" % name)
	print("a_demo: saved %s %s" % [name, game.net_status()])

func _process(delta: float) -> void:
	t += delta
	phase_t += delta
	if t - last_log > 5.0:
		last_log = t
		print("a_demo t=%.0f phase=%d %s" % [t, phase, game.net_status()])
	if t > 275.0:
		print("a_demo: timed out")
		get_tree().quit(1)
		return
	var hud = main.get_node("HUD")
	match phase:
		0:
			# the single-process game is up: record it
			if game.has_world() and not game.is_loading():
				print("a_demo: ", game.exec("record a_demo"))
				phase = 1
				phase_t = 0.0
		1:
			# the loopback session is active and recording
			if phase_t > 1.0 and game.has_world() and not game.is_loading() and game.net_status().contains("cl=Active"):
				hud.close_menu()
				print("a_demo: recording: ", FileAccess.file_exists("user://demos/a_demo.dm_71"), " ", game.exec("team humans"))
				phase = 2
				phase_t = 0.0
		2:
			if phase_t > 2.0:
				print("a_demo: ", game.exec("class rifle"))
				phase = 3
				phase_t = 0.0
		3:
			var h: Dictionary = game.get_hud()
			if h.get("alive", false) and phase_t > 2.0:
				hud.close_menu()
				game.exec("+forward")
				phase = 4
				phase_t = 0.0
		4:
			game.turn_view(40.0 * delta)
			if phase_t > 5.0:
				game.exec("-forward")
				await shot("a_demo_rec")
				print("a_demo: ", game.exec("stoprecord"))
				var f := FileAccess.open("user://demos/a_demo.dm_71", FileAccess.READ)
				print("a_demo: demo file bytes ", f.get_length() if f else -1)
				print("a_demo: ", game.exec("demo a_demo"))
				phase = 5
				phase_t = 0.0
		5:
			if game.has_world() and not game.is_loading():
				hud.close_menu()
				phase = 6
				phase_t = 0.0
		6:
			if phase_t > 1.5 * (shots + 1) and shots < 2:
				await shot("a_demo_play_%d" % shots)
				shots += 1
			if not game.has_world() and shots > 0:
				print("a_demo: demo completed %s" % game.net_status())
				get_tree().quit()
