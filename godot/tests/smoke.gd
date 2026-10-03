extends Node
## Headless smoke test: godot --headless --path godot res://tests/smoke.tscn
## Loads the real main scene, joins the humans as a rifleman, lets bots play
## and prints HUD + scoreboard snapshots, then exits with code 0/1.

var main: Node
var game
var t := 0.0
var step := 0
var ok := true

func _ready() -> void:
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	print("smoke: TremGame class = ", game.get_class())

func _process(delta: float) -> void:
	t += delta
	if step == 0 and t > 0.5:
		var e1: String = game.command("team humans")
		var e2: String = game.command("class rifle")
		print("smoke: join humans -> '%s' '%s'" % [e1, e2])
		step = 1
	elif step == 1 and t > 15.0:
		var h: Dictionary = game.get_hud()
		print("smoke: hud@15s ", h)
		if not h.alive:
			print("smoke: FAIL player did not spawn")
			ok = false
		print("smoke: camera node count = ", main.get_node("Game").get_child_count())
		step = 2
	elif step == 2 and t > 20.0:
		print("smoke: hud@20s ", game.get_hud())
		for r in game.get_scores():
			print("smoke: score ", r)
		print("smoke: armoury menu ", game.menu_items("armoury").size(), " rows")
		print("smoke: ", "PASS" if ok else "FAIL")
		get_tree().quit(0 if ok else 1)
