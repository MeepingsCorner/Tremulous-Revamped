extends Node
## Diagnostic: host a listen server like the Create Server menu does, then
## query it over UDP on every local IPv4 address and by LAN broadcast, the
## way a friend's game would find and reach it. Log: res://shots/lan_log.txt
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
	logf = FileAccess.open(ProjectSettings.globalize_path("res://shots/lan_log.txt"), FileAccess.WRITE)
	main = load("res://main.tscn").instantiate()
	add_child(main)
	game = main.get_node("Game")
	_run()

func _wait(s: float) -> void:
	await get_tree().create_timer(s).timeout

func _query(ip: String, port: int, bcast: bool) -> void:
	var u := PacketPeerUDP.new()
	u.set_broadcast_enabled(bcast)
	var e := u.bind(0)
	if e != OK:
		say("lan: bind err %d" % e)
		return
	u.set_dest_address(ip, port)
	var pkt := PackedByteArray([255, 255, 255, 255])
	pkt.append_array("getinfo xxx".to_ascii_buffer())
	u.put_packet(pkt)
	var t := 0.0
	var got := false
	while t < 2.0:
		await _wait(0.1)
		t += 0.1
		while u.get_available_packet_count() > 0:
			var r := u.get_packet()
			got = true
			say("lan: %s:%d -> reply from %s: %s" % [ip, port, u.get_packet_ip(), r.slice(4, 120).get_string_from_ascii().replace("\n", " | ")])
	if not got:
		say("lan: %s:%d -> NO REPLY" % [ip, port])
	u.close()

func _run() -> void:
	while game.is_loading():
		await _wait(0.5)
	await _wait(1.0)
	game.exec("set net_enabled 1\nmap atcs\n")
	var t := 0.0
	while t < 60.0 and not game.has_world():
		await _wait(0.5)
		t += 0.5
	await _wait(3.0)
	say("lan: world=%s status=%s" % [game.has_world(), game.net_status()])
	say("lan: net_port=%s net_enabled=%s net_ip=%s sv_maxclients=%s" % [game.console("net_port"), game.console("net_enabled"), game.console("net_ip"), game.console("sv_maxclients")])
	for a in IP.get_local_addresses():
		if a.contains(":"):
			continue
		say("lan: local address " + a)
	for a in IP.get_local_addresses():
		if a.contains(":") or a.begins_with("169.254"):
			continue
		await _query(a, 30720, false)
	await _query("255.255.255.255", 30720, true)
	say("lan: done")
	await _wait(1.0)
	get_tree().quit()
