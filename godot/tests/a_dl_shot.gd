extends Node
## wt-a (dec037): a Godot client joins a headless Godot dedicated server
## that runs a map the client lacks: it downloads the map's pk3 into
## user://base (cl_allowDownload 1), restarts its search path, loads the map
## and passes the sv_pure check; screenshots user://a_dl_<n>.png.
## server: godot --headless --path godot -- --base=DIR +set dedicated 1 +set sv_allowDownload 1 +set net_port 30770 +map dltest
## client: xvfb-run -a godot --rendering-driver opengl3 --path godot res://tests/a_dl_shot.tscn -- --port=30770
## With --prompt the client keeps cl_allowDownload 0: CL_NextDownload asks the
## ui (download_popmenu, user://a_dl_prompt.png) and its "Download" answer
## (uiScript downloadUDP: com_downloadPrompt 4) starts the download.
var prompt := false
var prompted := false
var main: Node
var game
var port := 30770
var t := 0.0
var phase := 0
var phase_t := 0.0
var last_log := 0.0
var shots := 0
var map := "dltest"
var dl_done := -1.0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--port="):
			port = int(a.substr(7))
		elif a.begins_with("--map="):
			map = a.substr(6)
		elif a == "--prompt":
			prompt = true
	# the client must lack the map (a previous run's download goes)
	var d := DirAccess.open("user://base")
	if d:
		for f in d.get_files():
			if f.begins_with("map-%s" % map):
				d.remove(f)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	print("a_dl: ", game.exec("set cl_allowDownload %d\nset name Downloader\nconnect 127.0.0.1:%d" % [0 if prompt else 1, port]))

func _process(delta: float) -> void:
	t += delta
	phase_t += delta
	if t - last_log > 5.0:
		last_log = t
		print("a_dl t=%.0f phase=%d %s" % [t, phase, game.net_status()])
	if t > 270.0:
		print("a_dl: timed out")
		get_tree().quit(1)
		return
	var hud = main.get_node("HUD")
	if dl_done < 0.0 and _downloaded_pk3() != "":
		dl_done = t
		var sz := FileAccess.get_file_as_bytes("user://base/" + _downloaded_pk3()).size()
		print("a_dl: %s (%d bytes) downloaded %.2f s after connecting: %.0f KB/s" % [_downloaded_pk3(), sz, t, sz / 1024.0 / max(t, 0.001)])
	match phase:
		0:
			var menus = hud.get_node("TremMenus")
			if prompt and not prompted and menus.get_cvar("com_downloadPromptText") != "":
				prompted = true
				phase_t = 0.0
				print("a_dl: the ui asks: ", menus.get_cvar("com_downloadPromptText"))
			if prompt and prompted and phase_t > 2.0 and phase_t < 100.0:
				await RenderingServer.frame_post_draw
				get_viewport().get_texture().get_image().save_png("user://a_dl_prompt.png")
				print("a_dl: saved a_dl_prompt; answering Download (downloadUDP)")
				menus.set_cvar("com_downloadPrompt", "4")
				menus.set_cvar("com_downloadPromptText", "")
				hud.close_menu()
				phase_t = 100.0
			if game.has_world() and not game.is_loading():
				hud.close_menu()
				var got := _downloaded_pk3() != ""
				print("a_dl: in the game, downloaded pk3: %s %s" % [got, game.net_status()])
				if not got:
					get_tree().quit(1)
					return
				print("a_dl: ", game.exec("team humans"))
				phase = 1
				phase_t = 0.0
		1:
			if phase_t > 3.0:
				print("a_dl: ", game.exec("class rifle"))
				phase = 2
				phase_t = 0.0
		2:
			var h: Dictionary = game.get_hud()
			if h.get("alive", false) and phase_t > 3.0:
				hud.close_menu()
				phase = 3
				phase_t = 0.0
		3:
			if phase_t > 2.0:
				await RenderingServer.frame_post_draw
				get_viewport().get_texture().get_image().save_png("user://a_dl_%d.png" % shots)
				print("a_dl: saved %d %s" % [shots, game.net_status()])
				shots += 1
				phase_t = 0.0
				game.turn_view(120.0)
				if shots >= 2:
					print("a_dl: done")
					get_tree().quit()


func _downloaded_pk3() -> String:
	var d := DirAccess.open("user://base")
	if d:
		for f in d.get_files():
			if f.begins_with("map-%s" % map) and f.ends_with(".pk3"):
				return f
	return ""
