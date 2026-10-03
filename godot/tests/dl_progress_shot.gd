extends Node
## The connect screen while a download runs, joined the way the server
## browser joins (uiScript JoinServer: "connect ADDR -PROTOCOL") from the
## main menu: screenshots user://dlp_<n>.png every 2 s until in the game.
## godot --path godot res://tests/dl_progress_shot.tscn -- --main-menu --addr=HOST:PORT --protocol=71
var main: Node
var game
var hud
var addr := "127.0.0.1:30770"
var protocol := "71"
var shots := 0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--addr="):
			addr = a.substr(7)
		elif a.begins_with("--protocol="):
			protocol = a.substr(11)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	hud = main.get_node("HUD")
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot() -> void:
	await RenderingServer.frame_post_draw
	var path := "user://dlp_%d.png" % shots
	get_viewport().get_texture().get_image().save_png(path)
	print("dl_progress_shot: %d connecting=%s loading=%s %s -> %s" % [shots, game.is_connecting(), game.is_loading(), game.net_status(), ProjectSettings.globalize_path(path)])
	shots += 1

func _run() -> void:
	# --skip-intro: from the main menu, as the server browser is reached
	if "--skip-intro" in OS.get_cmdline_user_args():
		await _wait(1.2)
		var ev := InputEventKey.new()
		ev.keycode = KEY_SPACE
		ev.physical_keycode = KEY_SPACE
		ev.pressed = true
		Input.parse_input_event(ev)
		await get_tree().process_frame
		var up := ev.duplicate()
		up.pressed = false
		Input.parse_input_event(up)
	await _wait(2.0)
	print("dl_progress_shot: ", game.exec("set cl_allowDownload 1\nconnect %s -%s" % [addr, protocol]))
	for i in 12:
		await _wait(2.0)
		await _shot()
		if game.has_world() and not game.is_loading():
			break
	game.exec("disconnect")
	await _wait(0.5)
	get_tree().quit()
