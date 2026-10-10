extends Node
## Diagnostic: join a live 1.1 server, spawn, fire, and trace the events
## (cg_debugEvents with time / predicted) and the ping. Log: res://shots/fire_log.txt
var main: Node
var game
var logf: FileAccess
const SERVER := "104.236.81.57:30720"

func say(msg: String) -> void:
	print(msg)
	if logf:
		logf.store_line(msg)
		logf.flush()

func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("res://shots"))
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/fire_log.txt"), FileAccess.WRITE)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _run() -> void:
	while game.is_loading():
		await _wait(0.5)
	await _wait(1.0)
	var f := FileAccess.open("user://autogen.cfg", FileAccess.READ)
	if f:
		for l in f.get_as_text().split("\n"):
			if l.contains(" name ") or l.contains(" rate ") or l.contains("r_width") or l.contains("cl_maxpackets") or l.contains(" snaps "):
				say("autogen: " + l)
	game.exec("name PortTest%d\n" % (randi() % 1000))
	say("connect: " + game.console("connect " + SERVER))
	var t := 0.0
	while t < 30.0 and not game.has_world():
		await _wait(0.5)
		t += 0.5
	await _wait(2.0)
	say("status: " + game.net_status())
	say("team: " + game.command("team humans"))
	await _wait(1.0)
	say("class: " + game.command("class rifle"))
	await _wait(1.5)
	var m1: int = game.key_number("MOUSE1")
	say("menu input_enabled=%s" % game.input_enabled)
	game.input_enabled = true
	game.key_event(m1, true)
	await _wait(0.2)
	game.key_event(m1, false)
	await _wait(3.0)
	say("status after spawn: " + game.net_status() + " input_enabled=%s" % game.input_enabled)
	game.input_enabled = true
	game.exec("set cg_debugEvents 1\n")
	say("fire start t=%d" % Time.get_ticks_msec())
	game.key_event(m1, true)
	await _wait(1.2)
	game.key_event(m1, false)
	say("fire end t=%d" % Time.get_ticks_msec())
	await _wait(1.0)
	game.exec("set cg_debugEvents 0\n")
	say("score: " + game.command("score"))
	await _wait(2.0)
	say("status after scores: " + game.net_status())
	say("disconnect: " + game.console("disconnect"))
	game.exec("name Player\n")
	await _wait(25.0)
	get_tree().quit()
