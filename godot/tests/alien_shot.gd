extends Node
## Diagnostic: advanced marauder skin and a dretch on a wall, third person.
## Screenshots and log in res://shots/ (alien_*.png, alien_log.txt).
var main: Node
var game
var logf: FileAccess

func say(msg: String) -> void:
	print(msg)
	if logf:
		logf.store_line(msg)
		logf.flush()

func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("res://shots"))
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/alien_log.txt"), FileAccess.WRITE)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	_run()

func _cmd(c: String) -> String:
	var r: String = game.command(c)
	say("alien_shot: %s -> %s" % [c, "ok" if r == "" else r])
	return r

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _shot(nm: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	var p := ProjectSettings.globalize_path("res://shots/" + nm + ".png")
	get_viewport().get_texture().get_image().save_png(p)
	say("alien_shot: saved " + p)

func _alive() -> void:
	var t := 0.0
	while t < 30.0:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			return
		await _wait(0.25)
		t += 0.25

func _run() -> void:
	while game.is_loading():
		await _wait(0.5)
	game.set_cheats(true)
	await _wait(0.5)
	_cmd("team aliens")
	_cmd("class level0")
	main.get_node("HUD").close_menu()
	await _alive()
	await _wait(1.0)
	game.input_enabled = true
	game.exec("set cg_thirdPerson 1\nset cg_thirdPersonRange 90\n")
	# walk into a wall and up it: a dretch climbs whatever it walks into
	for i in range(4):
		var yaw := i * 90
		_cmd("setviewpos -1800 300 60 %d" % yaw)
		await _wait(0.3)
		game.exec("+forward\n")
		await _wait(2.5)
		game.exec("-forward\n")
		await _wait(0.4)
		var h: Dictionary = game.get_hud()
		say("alien_shot: dretch try %d hud=%s" % [i, str(h.get("wallclimbing", "?"))])
		for line in game.actor_info():
			if line.contains("ent 0 "):
				say("actor: " + line)
		await _shot("alien_dretch_%d" % i)
	# advanced marauder
	_cmd("evolve level2upg")
	_cmd("class level2upg")
	await _wait(3.0)
	_cmd("setviewpos -1800 300 60 0")
	await _wait(0.8)
	await _shot("alien_adv_mara_0")
	await _wait(0.5)
	await _shot("alien_adv_mara_1")
	game.exec("set cg_thirdPerson 0\n")
	get_tree().quit()
