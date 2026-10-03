extends Node
## Diagnostic: join real internet servers and log each connection step.
## Log: res://shots/net_log.txt (plus the game's own console prints on stdout).
var main: Node
var game
var logf: FileAccess
const SERVERS := ["104.236.81.57:30720", "66.94.98.116:30722"]

func say(msg: String) -> void:
	print(msg)
	if logf:
		logf.store_line(msg)
		logf.flush()

func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("res://shots"))
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/net_log.txt"), FileAccess.WRITE)
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
	for s in SERVERS:
		say("net_test: ---- connect %s -> '%s'" % [s, game.console("connect " + s)])
		var last := ""
		for t in range(20):
			await _wait(1.0)
			var st: String = "%s connecting=%s world=%s loading=%s" % [game.net_status(), game.is_connecting(), game.has_world(), game.is_loading()]
			if st != last:
				say("net_test: t=%ds %s" % [t, st])
				last = st
		say("net_test: disconnect -> '%s'" % game.console("disconnect"))
		await _wait(2.0)
	say("net_test: done")
	await _wait(5.0)
	get_tree().quit()
