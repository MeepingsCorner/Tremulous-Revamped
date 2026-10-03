extends Node
## The server browser's LAN list with a vanilla 1.1.0 server and a port
## (1.3 / legacy) server on the network (dec040): both listed with their
## version, then the "Version:" filter set to 1.1 and to 1.3.
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/lan_browser_shot.tscn -- --main-menu
## -> user://b_lan_all.png, b_lan_11.png, b_lan_13.png
var main: Node
var game
var menus
var hud

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	hud = main.get_node("HUD")
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
	print("lan_browser_shot: %s connected=%s cinematic=%s menu=%s -> %s" % [kind, game.has_world(), menus.in_cinematic(), menus.current_menu(), ProjectSettings.globalize_path(path)])

## a click at a 640x480 virtual position
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
	print("lan_browser_shot: click %s/%s at %s" % [menu, item, c])
	if c.x < 0:
		return
	await _click(c.x, c.y)
	await _wait(0.3)

## hover first (mouseEnter swaps the arrow for its _alt button), then click
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
	await _wait(1.2)
	await _key(KEY_SPACE)
	await _wait(1.0)
	await _click_item("main", "Play")
	await _wait(1.0)
	# Source: Internet -> LAN (the MULTI item cycles on a click)
	await _click(80.0, 31.0)
	await _wait(4.0)
	await _shot("lan_all")
	# Version: All -> 1.3 -> 1.1 (the MULTI item; its action runs RefreshFilter)
	await _click(385.0, 63.0)
	await _wait(3.0)
	await _shot("lan_13")
	await _click(385.0, 63.0)
	await _wait(3.0)
	await _shot("lan_11")
	await _click(385.0, 63.0)
	await _wait(0.5)
	get_tree().quit()
