extends Node
## Screenshots of the wt-misc pieces on niveus (curved snow patches):
## xvfb-run godot --rendering-driver opengl3 --path godot res://tests/misc_shot.tscn
## -> user://m_warmup.png   (g_doWarmup 1: CG_DrawWarmup's countdown while nobody spawns)
##    user://m_console.png  (the drop-down console: printed lines, history, a Tab
##                           completion listing and the completed edit line)
##    user://m_testmodel.png (testmodel: an MD3 100 units in front of the view)
##    user://m_mark_patch.png (rifle marks clipped to a curved patch: R_SubdividePatchToGrid)
var main: Node
var game
var hud

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	hud = main.get_node("HUD")
	game.set_cheats(true)
	_run()

func _log(s: String) -> void:
	print("misc_shot: ", s)

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	_log(c + " -> " + ("ok" if r == "" else r))
	return r

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _wait_loaded(limit: float) -> void:
	var t := 0.0
	while game.is_loading() and t < limit:
		await _wait(0.5)
		t += 0.5

func _wait_alive(limit: float) -> bool:
	var t := 0.0
	while t < limit:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			return true
		await _wait(0.25)
		t += 0.25
	_log("not alive after %.0f s" % limit)
	return false

func _shot(path: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png(path)
	_log("saved " + ProjectSettings.globalize_path(path))

## a key press and release for the focused console (keycodes + unicode)
func _key(code: int, uni: int = 0, ctrl := false) -> void:
	for down in [true, false]:
		var k := InputEventKey.new()
		k.keycode = code
		k.physical_keycode = code
		k.unicode = uni if down else 0
		k.pressed = down
		k.ctrl_pressed = ctrl
		Input.parse_input_event(k)
	await _wait(0.03)

func _type(text: String) -> void:
	for c in text:
		var u := c.unicode_at(0)
		var code := u
		if c >= "a" and c <= "z":
			code = KEY_A + (u - 97)
		elif c == " ":
			code = KEY_SPACE
		elif c == "/":
			code = KEY_SLASH
		elif c == "_":
			code = KEY_UNDERSCORE
		await _key(code, u)

func _join(team: String, cls: String) -> void:
	_cmd("team " + team)
	_cmd("class " + cls)
	hud.close_menu()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _fire(secs: float) -> void:
	var mb := InputEventMouseButton.new()
	mb.button_index = MOUSE_BUTTON_LEFT
	mb.pressed = true
	Input.parse_input_event(mb)
	await _wait(secs)
	mb = InputEventMouseButton.new()
	mb.button_index = MOUSE_BUTTON_LEFT
	mb.pressed = false
	Input.parse_input_event(mb)

## the name cvar's value ("name" is:"X^7" ...)
func _name() -> String:
	var r: String = game.exec("name")
	return r.get_slice("is:\"", 1).get_slice("^7\"", 0)

func _run() -> void:
	await _wait(0.3)
	# the user folder (autogen.cfg) is shared: the name is put back at the end
	var old_name := _name()
	# warmup on the next map (the g_ cvars outlive maps on the host)
	_log(game.exec("set g_doWarmup 1"))
	_log(game.exec("set g_warmup 25"))
	_log("load niveus: " + game.load_map(hud.pk3_paths(), "niveus"))
	await _wait_loaded(150.0)
	await _wait(0.5)
	game.set_cheats(true)
	for i in 3:
		game.remove_bot("aliens")
		game.remove_bot("humans")
	_join("humans", "rifle")
	await _wait(1.5)
	_log("hud during warmup: alive %s queued %s" % [game.get_hud().get("alive", false), game.get_hud().get("queued", false)])
	await _shot("user://m_warmup.png")
	# the console: a command, a chat-free history line, Tab completion
	hud._on_host_command("toggleconsole")
	await _wait(0.4)
	await _type("/echo hello from the console")
	await _key(KEY_ENTER)
	await _type("/set name ConsoleTester")
	await _key(KEY_ENTER)
	await _type("cl_")
	await _key(KEY_TAB)
	await _type("fr")
	await _key(KEY_TAB)
	await _wait(0.6)
	var con: Control = hud.console
	_log("console rect %s visible %s down %s in tree %s" % [con.get_rect(), con.visible, con.is_down(), con.is_inside_tree()])
	await _shot("user://m_console.png")
	# history: up arrow brings back the last line
	await _key(KEY_ESCAPE)
	await _wait(0.4)
	_log("name now: " + game.exec("name"))
	# spawn once the warmup is over
	await _wait_alive(60.0)
	await _wait(0.5)
	# testmodel in front of the view, the bounding boxes of the structures
	_log(game.exec("testmodel models/buildables/reactor/reactor.md3"))
	_log(game.exec("nextframe"))
	await _wait(0.8)
	await _shot("user://m_testmodel.png")
	_log(game.exec("testmodel"))
	# a curved snow patch: stand above it and shoot down
	_cmd("setviewpos 352 -2176 -150 0")
	await _wait(0.8)
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
	game.input_enabled = true
	for i in 60:
		var f: PackedStringArray = game.console("cg_viewdebug").split(" ")
		if f.size() > 5 and float(f[5]) >= 58.0:
			break
		var m := InputEventMouseMotion.new()
		m.relative = Vector2(0, 12)
		Input.parse_input_event(m)
		await _wait(0.05)
	await _fire(2.0)
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	await _wait(0.4)
	await _shot("user://m_mark_patch.png")
	_log("view " + game.console("cg_viewdebug"))
	game.exec("seta name \"%s\"" % old_name)
	await _wait(1.5)
	get_tree().quit()
