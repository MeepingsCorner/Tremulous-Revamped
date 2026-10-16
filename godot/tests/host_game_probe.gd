extends Node
## Host Game through the real menus (Play -> Create Server -> Create), on a fresh profile:
## godot --path godot res://tests/host_game_probe.tscn -- --main-menu --port=30777 [--hold=60]
## Logs the net status once the world is up, then holds so a second client can join.
var main: Node
var game
var menus
var port := 30777
var hold := 60.0

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--port="):
			port = int(a.substr(7))
		if a.begins_with("--hold="):
			hold = float(a.substr(7))
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	menus = main.get_node("HUD/TremMenus")
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

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
	print("host_probe: click %s/%s at %s" % [menu, item, c])
	if c.x < 0:
		return
	await _click(c.x, c.y)
	await _wait(0.3)

func _hover_click(menu: String, item: String, alt: String) -> void:
	var c: Vector2 = menus.item_center(menu, item)
	var size := get_viewport().get_visible_rect().size
	var mv := InputEventMouseMotion.new()
	mv.position = Vector2(c.x * size.x / 640.0, c.y * size.y / 480.0)
	mv.global_position = mv.position
	Input.parse_input_event(mv)
	await _wait(0.2)
	await _click_item(menu, alt)

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

func _run() -> void:
	await _wait(1.5)
	await _key(KEY_SPACE)
	await _wait(1.0)
	print("host_probe: menu=", menus.current_menu(), " ", game.exec("set net_upnp 0\nset net_port %d" % port))
	await _click_item("main", "Play")
	await _wait(0.5)
	await _click_item("joinserver", "createServer")
	await _wait(0.5)
	await _hover_click("createserver", "accept", "accept_alt")
	var t := 0.0
	while not game.has_world() and t < 120.0:
		await _wait(0.5)
		t += 0.5
	await _wait(3.0)
	print("host_probe: world=%s status=%s" % [game.has_world(), String(game.net_status())])
	await _wait(hold)
	print("host_probe: end status=", String(game.net_status()))
	get_tree().quit()
