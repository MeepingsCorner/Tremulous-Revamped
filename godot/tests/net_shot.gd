extends Node
## Two Godot instances over UDP (dec033): a listen server ("--role=server")
## and a client ("--role=client") join the humans, stand face to face on
## ATCS and save what each one sees (user://net_<role>_<n>.png); the client
## then shoots.
## xvfb-run -a godot --rendering-driver opengl3 --path godot res://tests/net_shot.tscn -- --role=server --port=30760
## xvfb-run -a godot --rendering-driver opengl3 --path godot res://tests/net_shot.tscn -- --role=client --port=30760
var main: Node
var game
var role := "server"
var port := 30760
var t := 0.0
var phase := 0
var phase_t := 0.0
var shots := 0
var last_log := 0.0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--role="):
			role = a.substr(7)
		elif a.begins_with("--port="):
			port = int(a.substr(7))
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	if role == "server":
		print("net_shot: ", game.exec("set net_enabled 1\nset net_port %d\nset sv_hostname NetShot\nset name Hosty\ndevmap atcs" % port))
	else:
		print("net_shot: ", game.exec("set name Visitor\nconnect 127.0.0.1:%d" % port))

func _process(delta: float) -> void:
	t += delta
	phase_t += delta
	if t - last_log > 5.0:
		last_log = t
		print("net_shot[%s] t=%.0f phase=%d %s" % [role, t, phase, game.net_status()])
	if t > 270.0:
		print("net_shot[%s]: timed out" % role)
		get_tree().quit(1)
		return
	var hud = main.get_node("HUD")
	match phase:
		0:
			if game.has_world() and not game.is_loading():
				hud.close_menu()
				if role == "server":
					# both join the humans: let the client's rifle hurt the host
					game.exec("g_friendlyFire 1")
				print("net_shot[%s]: in the game: %s" % [role, game.exec("team humans")])
				phase = 1
				phase_t = 0.0
		1:
			if phase_t > 3.0:
				print("net_shot[%s]: %s" % [role, game.exec("class rifle")])
				phase = 2
				phase_t = 0.0
		2:
			var h: Dictionary = game.get_hud()
			if h.get("alive", false) and phase_t > 2.0:
				hud.close_menu()
				# back to back in the corridor between the telenodes and the
				# medistat (TeleportPlayer pushes them apart), then turn round
				var pos := "1700 260 60 180" if role == "server" else "1800 260 60 0"
				print("net_shot[%s]: setviewpos %s" % [role, game.exec("setviewpos " + pos)])
				phase = 3
				phase_t = 0.0
		3:
			# both stand still a while (the other one may still be spawning)
			if phase_t > 6.0:
				game.turn_view(180.0)
				phase = 6
				phase_t = 0.0
		6:
			if phase_t > 6.0:
				hud.close_menu()
				phase = 4
				phase_t = 0.0
		4:
			if phase_t > 2.0 * (shots + 1):
				await RenderingServer.frame_post_draw
				get_viewport().get_texture().get_image().save_png("user://net_%s_%d.png" % [role, shots])
				print("net_shot[%s]: saved %d %s" % [role, shots, game.net_status()])
				shots += 1
				if role == "client" and shots == 2:
					game.exec("+attack")
				if role == "client" and shots == 4:
					game.exec("-attack")
				if shots >= 5:
					phase = 5
					phase_t = 0.0
		5:
			if phase_t > 3.0:
				print("net_shot[%s]: done %s" % [role, game.net_status()])
				get_tree().quit()
