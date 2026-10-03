extends Node
## wt-b checks (dec038), saved as user://b2_*.png:
##  --mode=joy    the main menu and the console driven by joystick keys
##                (IN_JoyMove's K_* through the ui / console key catchers)
##  --mode=video  `video` capture on ATCS: videos/b2test.avi (cl_avi.c)
##  --mode=voip --role=server|client --port=N   two instances over UDP:
##                the host talks (a 440 Hz test tone as the microphone,
##                +voiprecord), the client hears it (CL_ParseVoip -> raw streams)
## xvfb-run -a godot --rendering-driver opengl3 --path godot res://tests/wtb_shot.tscn -- --mode=joy --main-menu
var main: Node
var game
var menus
var hud
var mode := "joy"
var role := "server"
var port := 30790

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--mode="):
			mode = a.substr(7)
		elif a.begins_with("--role="):
			role = a.substr(7)
		elif a.begins_with("--port="):
			port = int(a.substr(7))
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	hud = main.get_node("HUD")
	menus = main.get_node_or_null("HUD/TremMenus")
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	match mode:
		"joy":
			_joy()
		"video":
			_video()
		"voip":
			_voip()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(kind: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	var path := "user://b2_%s.png" % kind
	img.save_png(path)
	print("wtb_shot: %s -> %s" % [kind, ProjectSettings.globalize_path(path)])

func _key(code: Key, unicode: int = 0) -> void:
	var ev := InputEventKey.new()
	ev.keycode = code
	ev.physical_keycode = code
	ev.unicode = unicode
	ev.pressed = true
	Input.parse_input_event(ev)
	await get_tree().process_frame
	var up := ev.duplicate()
	up.pressed = false
	Input.parse_input_event(up)
	await get_tree().process_frame

func _joy_press(name: String) -> void:
	var k: int = game.key_number(name)
	game.joy_key_event(k, true)
	await _wait(0.15)
	game.joy_key_event(k, false)
	await _wait(0.15)

func _joy() -> void:
	await _wait(1.2)
	await _key(KEY_SPACE)  # skip the intro cinematic
	await _wait(1.0)
	var f0: String = menus.focused_item()
	print("wtb_shot: joy main menu=%s focus=%s" % [menus.current_menu(), f0])
	await _shot("joy_main_0")
	# K_JOY1..4: no default action in Menu_HandleKey
	await _joy_press("JOY1")
	var f1: String = menus.focused_item()
	# the stick's down arrow (joy_keys[]): Menu_SetNextCursorItem
	await _joy_press("DOWNARROW")
	var f2: String = menus.focused_item()
	await _joy_press("DOWNARROW")
	var f3: String = menus.focused_item()
	print("wtb_shot: joy focus %s -JOY1-> %s -DOWN-> %s -DOWN-> %s" % [f0, f1, f2, f3])
	await _shot("joy_main_1")
	var ok_menu := f1 == f0 and f2 != f1 and f3 != f2
	print("wtb_shot: joy result menu_ok=%s" % ok_menu)
	get_tree().quit(0 if ok_menu else 1)

## the console (in a game): a line in the history, then the stick's up
## arrow recalls it (Console_Key with KEYCATCH_CONSOLE)
func _joy_console() -> void:
	hud._on_host_command("toggleconsole")
	await _wait(0.5)
	for ch in "/echo joyhist":
		var u := ch.unicode_at(0)
		var code: int = KEY_A + (u - 97) if ch >= "a" and ch <= "z" else (KEY_SPACE if ch == " " else KEY_SLASH)
		await _key(code, u)
	await _key(KEY_ENTER)
	await _wait(0.3)
	await _joy_press("UPARROW")
	await _wait(0.5)
	await _shot("joy_console")
	hud._on_host_command("toggleconsole")
	await _wait(0.5)

func _video() -> void:
	# the default start is ATCS, single process
	var t := 0.0
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
		t += 0.5
		if t > 200:
			print("wtb_shot: video: no map")
			get_tree().quit(1)
			return
	hud.close_menu()
	await _wait(1.0)
	await _joy_console()
	print("wtb_shot: video cmd: [%s]" % game.exec("video b2demo"))
	print("wtb_shot: video_open: [%s]" % game.video_open("b2test"))
	var frames := 0
	var t0 := Time.get_ticks_msec()
	while frames < 30:
		await get_tree().process_frame
		frames += 1
	await _shot("video")
	print("wtb_shot: video: %d frames in %d ms, recording=%s" % [frames, Time.get_ticks_msec() - t0, game.video_recording()])
	var out: String = game.video_close()
	print("wtb_shot: video_close: [%s]" % out.strip_edges())
	var f := FileAccess.open("user://videos/b2test.avi", FileAccess.READ)
	var ok: bool = f != null and f.get_length() > 1000 and f.get_buffer(4).get_string_from_ascii() == "RIFF"
	print("wtb_shot: video file ok=%s size=%d" % [ok, f.get_length() if f else -1])
	get_tree().quit(0 if ok else 1)

func _voip() -> void:
	if role == "server":
		print("wtb_shot: ", game.exec("set net_enabled 1\nset net_port %d\nset sv_hostname VoipShot\nset name Talker\ndevmap atcs" % port))
	else:
		print("wtb_shot: ", game.exec("set name Listener\nconnect 127.0.0.1:%d" % port))
	var t := 0.0
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
		t += 0.5
		if t > 200:
			print("wtb_shot[%s]: no map" % role)
			get_tree().quit(1)
			return
	hud.close_menu()
	await _wait(3.0)
	print("wtb_shot[%s]: %s" % [role, game.exec("team humans")])
	if role == "server":
		# wait for the listener, then talk for a while
		await _wait(25.0)
		game.voip_test_tone(440.0, 8000.0)
		print("wtb_shot[server]: +voiprecord %s" % game.exec("+voiprecord"))
		await _wait(4.0)
		await _shot("voip_talker")
		await _wait(20.0)
		game.exec("-voiprecord")
		print("wtb_shot[server]: done talking")
		await _wait(20.0)
		get_tree().quit(0)
	else:
		var heard := ""
		var t2 := 0.0
		while t2 < 90.0:
			await _wait(2.0)
			t2 += 2.0
			heard = game.voip_stats()
			print("wtb_shot[client] t=%.0f voip_stats=[%s] %s" % [t2, heard, game.net_status()])
			if heard != "" and int(heard.split(":")[1]) > 48000:
				break
		await _shot("voip_listener")
		print("wtb_shot[client]: voip %s" % game.exec("voip gain 0"))
		var ok := heard != "" and int(heard.split(":")[1]) > 48000
		print("wtb_shot[client]: result heard=%s ok=%s" % [heard, ok])
		get_tree().quit(0 if ok else 1)
