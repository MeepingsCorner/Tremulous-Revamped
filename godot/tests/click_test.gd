extends Node
## Diagnostic: injects mouse motion + a left click on the main menu's "Play"
## item through Godot's input pipeline (no screen control) and logs what the
## menu does. Log: res://shots/click_log.txt
var main: Node
var menus
var logf: FileAccess

func say(msg: String) -> void:
	print(msg)
	if logf:
		logf.store_line(msg)
		logf.flush()

func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("res://shots"))
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/click_log.txt"), FileAccess.WRITE)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	menus = main.get_node_or_null("HUD/TremMenus")
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _mouse(pos: Vector2, pressed: bool, button: bool) -> void:
	if button:
		var b := InputEventMouseButton.new()
		b.button_index = MOUSE_BUTTON_LEFT
		b.pressed = pressed
		b.position = pos
		b.global_position = pos
		Input.parse_input_event(b)
	else:
		var m := InputEventMouseMotion.new()
		m.position = pos
		m.global_position = pos
		m.relative = Vector2(1, 1)
		Input.parse_input_event(m)

func _run() -> void:
	await _wait(6.0)
	var vs := get_viewport().get_visible_rect().size
	say("click_test: viewport %s menus=%s" % [str(vs), str(menus)])
	say("click_test: menu before: '%s'" % (menus.current_menu() if menus else "?"))
	say("click_test: menus size=%s gpos=%s visible=%s filter=%d mouse_mode=%d" % [str(menus.size), str(menus.global_position), str(menus.visible), menus.mouse_filter, Input.mouse_mode])
	var before: String = menus.current_menu()
	var hits := 0
	for gy in range(1, 24):
		for gx in range(1, 32):
			var p := Vector2(vs.x * gx / 32.0, vs.y * gy / 24.0)
			_mouse(p, false, false)
			await get_tree().process_frame
			_mouse(p, true, true)
			await get_tree().process_frame
			_mouse(p, false, true)
			await get_tree().process_frame
			var now: String = menus.current_menu()
			if now != before:
				say("click_test: menu '%s' -> '%s' after click at %s" % [before, now, str(p)])
				before = now
				hits += 1
				if hits >= 5:
					break
		if hits >= 5:
			break
	say("click_test: done, %d menu changes, final menu '%s'" % [hits, before])
	get_tree().quit()
