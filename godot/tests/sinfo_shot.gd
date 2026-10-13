extends Node
## dec058: LAN browser with two local servers (bots), select the first row,
## press Server Info, shoot the popup after 1, 3 and 6 seconds.
var main: Node
var game
var menus

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	menus = main.get_node("HUD/TremMenus")
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(kind: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	var path := "user://b_%s.png" % kind
	img.save_png(path)
	print("sinfo_shot: %s menu=%s -> %s" % [kind, menus.current_menu(), ProjectSettings.globalize_path(path)])

func _click(vx: float, vy: float) -> void:
	var size := get_viewport().get_visible_rect().size
	var pos := Vector2(vx * size.x / 640.0, vy * size.y / 480.0)
	var mv := InputEventMouseMotion.new()
	mv.position = pos
	mv.global_position = pos
	Input.parse_input_event(mv)
	await get_tree().process_frame
	for pressed in [true, false]:
		var ev := InputEventMouseButton.new()
		ev.button_index = MOUSE_BUTTON_LEFT
		ev.pressed = pressed
		ev.position = pos
		ev.global_position = pos
		Input.parse_input_event(ev)
		await get_tree().process_frame

func _click_item(menu: String, item: String) -> void:
	var c: Vector2 = menus.item_center(menu, item)
	print("sinfo_shot: click %s/%s at %s" % [menu, item, c])
	if c.x < 0:
		return
	await _click(c.x, c.y)
	await _wait(0.3)

func _key(code: Key) -> void:
	var ev := InputEventKey.new()
	ev.keycode = code
	ev.physical_keycode = code
	ev.pressed = true
	Input.parse_input_event(ev)
	await get_tree().process_frame
	var up := ev.duplicate()
	up.pressed = false
	Input.parse_input_event(up)
	await get_tree().process_frame

func _arg(k: String, d: String) -> String:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--%s=" % k):
			return a.substr(k.length() + 3)
	return d

func _server(port: int, name: String, bots: Array) -> int:
	var args := ["--headless", "--path", _arg("server", ""), "--", "+set", "net_upnp", "0", "+set", "sv_master1", "", "+set", "dedicated", "1", "+set", "sv_hostname", name, "+set", "net_port", str(port), "+map", "atcs"]
	for b in bots:
		for i in range(12):
			args.append("+wait")
		args.append_array(["+port_addbot", b])
	return OS.create_process(OS.get_executable_path(), args)

func _run() -> void:
	var pre := _arg("prefix", "x")
	var pids := [_server(30721, "Bots Alpha", ["humans", "humans", "aliens"]), _server(30722, "Bots Bravo", ["aliens"])]
	await _wait(1.2)
	await _key(KEY_SPACE)
	await _wait(1.0)
	await _click_item("main", "Play")
	await _click(80.0, 31.0)
	print("sinfo_shot: source LAN")
	await _wait(12.0)
	await _shot("%s_list" % pre)
	await _click(100.0, 143.0)
	await _wait(0.5)
	await _click_item("joinserver", "serverinfo")
	await _wait(1.0)
	await _shot("%s_info1" % pre)
	await _wait(2.0)
	await _shot("%s_info3" % pre)
	await _wait(3.0)
	await _wait(3.0)
	await _shot("%s_info6" % pre)
	# scroll the list down to the players
	for i in range(8):
		await _click(457.0, 335.0)
	await _wait(0.5)
	await _shot("%s_info_scrolled" % pre)
	for p in pids:
		OS.kill(p)
	get_tree().quit()
