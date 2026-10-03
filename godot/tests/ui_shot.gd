extends Node
## Diagnostic: screenshots of the in-game UI (team select, human/alien HUD,
## Esc menu, scoreboard) into res://shots/ui_*.png. Log: res://shots/ui_log.txt
var main: Node
var game
var hud
var menus
var logf: FileAccess

func say(msg: String) -> void:
	print(msg)
	if logf:
		logf.store_line(msg)
		logf.flush()

func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("res://shots"))
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/ui_log.txt"), FileAccess.WRITE)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	hud = main.get_node("HUD")
	menus = main.get_node_or_null("HUD/TremMenus")
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _shot(nm: String) -> void:
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	get_viewport().get_texture().get_image().save_png(ProjectSettings.globalize_path("res://shots/ui_%s.png" % nm))
	say("ui_shot: %s menu='%s'" % [nm, menus.current_menu() if menus else "?"])

func _alive(limit: float) -> void:
	var t := 0.0
	while t < limit:
		var h: Dictionary = game.get_hud()
		if not h.is_empty() and h.alive:
			return
		await _wait(0.25)
		t += 0.25

func _menu(kind: String, nm: String) -> void:
	hud.open_menu(kind)
	await _wait(0.8)
	await _shot(nm)
	hud.close_menu()
	await _wait(0.3)

func _run() -> void:
	while game.is_loading():
		await _wait(0.5)
	await _wait(1.5)
	say("cmd: " + game.command("team humans"))
	say("cmd: " + game.command("class ckit"))
	hud.close_menu()
	await _alive(30.0)
	await _wait(1.0)
	await _menu("build", "7_human_build")
	await _menu("armoury", "8_armoury")
	await _menu("ingame_options", "9_options")
	await _menu("main", "10_main")
	say("cmd: " + game.command("team spectate"))
	await _wait(1.2)
	say("cmd: " + game.command("team aliens"))
	say("cmd: " + game.command("class builder"))
	hud.close_menu()
	await _alive(30.0)
	await _wait(1.0)
	await _menu("build", "11_alien_build")
	await _menu("evolve", "12_evolve")
	await _shot("13_hud_granger")
	get_tree().quit()
