extends Node
## The News page (dec065): main menu -> News, waits for the downloads, saves
## user://b_news_<prefix>.png (and b_news_<prefix>_early.png right after opening).
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/news_shot.tscn -- --main-menu --prefix=after
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
	print("news_shot: %s menu=%s -> %s" % [kind, menus.current_menu(), ProjectSettings.globalize_path(path)])

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
	var prefix := "x"
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--prefix="):
			prefix = a.substr(9)
	await _wait(1.2)
	await _key(KEY_SPACE)
	await _wait(1.0)
	var c: Vector2 = menus.item_center("main", "News")
	print("news_shot: click News at ", c)
	await _click(c.x, c.y)
	await _wait(0.15)
	await _shot("news_%s_early" % prefix)
	await _wait(8.0)
	await _shot("news_%s" % prefix)
	get_tree().quit()
