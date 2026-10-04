extends Node
## Builds the 4K texture pack (hdtex.rs) from the pk3s in res://base and
## user://base with Real-ESRGAN:
## godot --path <project> res://tests/hd_build.tscn -- --esrgan=<dir with realesrgan-ncnn-vulkan.exe> [--target=4096] [--out=DIR]
## Already built textures are skipped, so it can be stopped and resumed.
var target := 4096
var esrgan := ""
var out := ""

func _ready() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--esrgan="):
			esrgan = a.substr(9)
		elif a.begins_with("--target="):
			target = int(a.substr(9))
		elif a.begins_with("--out="):
			out = a.substr(6)
	if out == "":
		out = ProjectSettings.globalize_path("user://hd")
	_run()

func _pk3s() -> PackedStringArray:
	var l := PackedStringArray()
	for dir in ["res://base", "user://base"]:
		var d := DirAccess.open(dir)
		if d == null:
			continue
		for f in d.get_files():
			if f.to_lower().ends_with(".pk3"):
				l.append(ProjectSettings.globalize_path(dir + "/" + f))
	return l

func _esrgan(work: String, src: String, dst: String) -> void:
	var exe := esrgan.path_join("realesrgan-ncnn-vulkan.exe")
	var t := Time.get_ticks_msec()
	# one image per run: a whole folder in one run came back with images
	# blank or swapped between files
	var files := DirAccess.get_files_at(work.path_join(src))
	var bad := 0
	for f in files:
		var code := OS.execute(exe, ["-i", work.path_join(src).path_join(f), "-o", work.path_join(dst).path_join(f), "-n", "realesrgan-x4plus", "-s", "4", "-f", "png", "-m", esrgan.path_join("models")], [], true)
		if code != 0:
			bad += 1
	print("hd: realesrgan ", src, " -> ", dst, ": ", files.size(), " images, ", bad, " failed, in ", (Time.get_ticks_msec() - t) / 1000, " s")

func _run() -> void:
	await get_tree().process_frame
	var work := ProjectSettings.globalize_path("user://hd_work")
	var b = TremHdBuilder.new()
	var pk3s := _pk3s()
	print("hd: ", pk3s.size(), " pk3 files, target ", target, ", out ", out)
	# an image that comes out empty is not written: the next round redoes it
	for round in 3:
		var n: int = b.prepare(pk3s, out, work, target)
		print("hd: round ", round + 1, ": ", n, " to do")
		if n == 0:
			break
		await _round(b, work, n)
	print("hd: done")
	get_tree().quit()

func _round(b, work: String, n: int) -> void:
	if n > 0:
		_esrgan(work, "a_in", "a_mid")
		_esrgan(work, "a_mid", "a_out")
		_esrgan(work, "b_in", "b_out")
		var t := Time.get_ticks_msec()
		for i in n:
			var line: String = b.finish(i)
			if line != "":
				print(line, "  [", i + 1, "/", n, "]")
			if i % 2 == 0:
				await get_tree().process_frame
		print("hd: compressed ", n, " textures in ", (Time.get_ticks_msec() - t) / 1000, " s")
