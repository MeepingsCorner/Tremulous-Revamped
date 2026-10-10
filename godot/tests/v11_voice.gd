extends Node
## Voice chat on an original Tremulous 1.1 server (no VoIP): connect to
## 127.0.0.1:V11_PORT, join, press the microphone toggle, read the result.
## V11_PORT=30900 godot --path godot res://tests/v11_voice.tscn
var main: Node
var game
var hud

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	game.bots_per_team = 0
	hud = main.get_node("HUD")
	_run()

func _wait(secs: float) -> void:
	await get_tree().create_timer(secs).timeout

func _run() -> void:
	await _wait(4.0)
	var port := OS.get_environment("V11_PORT")
	print("v11v: ", game.exec("set name Checker\nconnect 127.0.0.1:%s" % port))
	var t := 0.0
	while not (game.has_world() and not game.is_loading()):
		await _wait(0.5)
		t += 0.5
		if t > 90:
			print("v11v: no map ", game.net_status())
			get_tree().quit(1)
			return
	await _wait(3.0)
	hud.close_menu()
	game.exec("team humans")
	await _wait(1.0)
	game.exec("class rifle")
	await _wait(3.0)
	hud.close_menu()
	print("v11v: before ", game.voip_hud_state().get_slice(" power", 0))
	print("v11v: toggle ", game.exec("voip_toggle"))
	await _wait(3.0)
	print("v11v: after ", game.voip_hud_state().get_slice(" power", 0), " ", game.voip_hud_state().get_slice(" hud=", 1))
	get_tree().quit(0)
