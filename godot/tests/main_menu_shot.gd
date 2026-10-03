extends Node
## The original main menu (UIMENU_MAIN, ui/menus.txt) as the start screen:
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/main_menu_shot.tscn -- --main-menu
## -> user://b_intro.png (cinematic splash.RoQ), b_main.png (the splash
##    model), b_join.png (server browser, offline), b_create.png and
##    b_create_map.png (createserver: FEEDER_MAPS + levelshot), b_options.png
##    (simple_options), b_gamma.png (r_gamma 1.6 pass), b_quit.png,
##    b_ingame.png (Create -> map), b_left.png (Leave -> main),
##    b_credits.png (quitCredit)
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
	print("main_menu_shot: %s connected=%s cinematic=%s menu=%s -> %s" % [kind, game.has_world(), menus.in_cinematic(), menus.current_menu(), ProjectSettings.globalize_path(path)])

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
	print("main_menu_shot: click %s/%s at %s" % [menu, item, c])
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
	var had_cfg := FileAccess.file_exists("user://autogen.cfg")
	var saved_cfg := FileAccess.get_file_as_string("user://autogen.cfg") if had_cfg else ""
	await _wait(1.2)
	print("main_menu_shot: start connected=%s cinematic=%s" % [game.has_world(), menus.in_cinematic()])
	await _shot("intro")
	# a key skips the cinematic (CL_KeyDownEvent -> escape -> UIMENU_MAIN)
	await _key(KEY_SPACE)
	await _wait(1.0)
	await _shot("main")
	# Play -> joinserver (InitServerList: no master server to answer)
	await _click_item("main", "Play")
	await _wait(0.5)
	await _shot("join")
	await _click_item("joinserver", "createServer")
	await _wait(0.5)
	await _shot("create")
	# the second map of FEEDER_MAPS
	var lc: Vector2 = menus.item_center("createserver", "maplist")
	await _click(lc.x, lc.y - 60.0)
	await _wait(0.4)
	await _shot("create_map")
	# back to main (Esc: createserver's onEsc, joinserver's closeJoin; while
	# a refresh still waits for a master server's answer, closeJoin stops it
	# first and the next Esc closes the browser, dec035)
	await _key(KEY_ESCAPE)
	await _wait(0.2)
	for i in 3:
		if menus.current_menu() == "main":
			break
		await _key(KEY_ESCAPE)
		await _wait(0.3)
	print("main_menu_shot: back at ", menus.current_menu())
	await _click_item("main", "Options")
	await _wait(0.4)
	await _shot("options")
	# Video Brightness: r_gamma through the client cvar (R_SetColorMappings pass)
	game.exec("r_gamma 1.6")
	await _wait(0.4)
	await _shot("gamma")
	game.exec("r_gamma 1")
	await _click_item("simple_options", "OK")
	await _click_item("main", "Quit")
	await _wait(0.3)
	await _shot("quit")
	await _click_item("quit_popmenu", "NO")
	# Play -> Create Server -> Create: StartServer runs `map <name>`
	await _click_item("main", "Play")
	await _click_item("joinserver", "createServer")
	await _hover_click("createserver", "accept", "accept_alt")
	var t := 0.0
	while not game.has_world() and t < 120.0:
		await _wait(0.5)
		t += 0.5
	await _wait(3.0)
	await _shot("ingame")
	# Esc -> ingame -> Leave -> yes: disconnect, back to the main menu
	hud.close_menu()
	await _wait(0.3)
	await _key(KEY_ESCAPE)
	await _wait(0.4)
	await _shot("ingame_bar")
	await _click_item("ingame", "Exit")
	await _wait(0.4)
	print("main_menu_shot: leave menu=", menus.current_menu())
	await _click_item("ingame_leave", "Main Menu")
	await _click_item("ingame_leave", "Yes")
	await _wait(1.5)
	await _shot("left")
	await _click_item("main", "Quit")
	await _click_item("quit_popmenu", "YES")
	await _wait(0.5)
	await _shot("credits")
	# put autogen.cfg back
	if had_cfg:
		var f := FileAccess.open("user://autogen.cfg", FileAccess.WRITE)
		f.store_string(saved_cfg)
		f.close()
	else:
		DirAccess.remove_absolute(ProjectSettings.globalize_path("user://autogen.cfg"))
	get_tree().quit()
