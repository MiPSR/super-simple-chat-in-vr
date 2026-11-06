extends SceneTree
## Bounded headless smoke stress, not a substitute for extended real VR sessions.
const Avatar = preload("res://scripts/avatar.gd")
var started := 0
var cycles := 100

func _initialize() -> void:
	started = Time.get_ticks_msec()
	var requested := int(OS.get_environment("SSC_STRESS_CYCLES"))
	if requested > 0:
		cycles = clampi(requested, 1, 2000)
	call_deferred("_run")

func _run() -> void:
	var path := "user://lifecycle-%s.zip" % Time.get_ticks_usec()
	var zip := ZIPPacker.new()
	assert(zip.open(path) == OK)
	zip.start_file("audio/tone.opus")
	zip.write_file(FileAccess.get_file_as_bytes(OS.get_environment("SSC_TEST_OPUS")))
	zip.close_file()
	zip.start_file("scripts/audio.luau")
	zip.write_file('avatar.after(600, function() end); avatar.on("action", function() avatar.play_sound("tone") end)'.to_utf8_buffer())
	zip.close_file()
	zip.close()
	var declaration := {"sounds":[{"id":"tone","path":"audio/tone.opus","loop":true}],"scripts":[{"path":"scripts/audio.luau","side":"client","attachment":"avatar","capabilities":["audio"]}]}
	var avatar := Avatar.new()
	root.add_child(avatar)
	var failures: Array[String] = []
	avatar.runtime_status.connect(func(message: String): failures.append(message))
	var baseline_nodes := -1
	var baseline_objects := -1
	for iteration in cycles:
		var old_decoder: RefCounted
		if avatar.model:
			old_decoder = avatar.model.get_meta("ssc_audio").clips.tone.decoder
		avatar.set_model(Node3D.new(), path, declaration)
		if old_decoder:
			assert(is_zero_approx(old_decoder.duration()), "Retired avatar retained an Opus decoder")
		old_decoder = null
		avatar.creator.dispatch("action", {"name":"play"})
		await process_frame
		await process_frame
		assert(failures.is_empty(), "Repeated load failed: " + str(failures))
		var audio: Node = avatar.model.get_meta("ssc_audio")
		assert(audio.clips.size() == 1 and audio.clips.tone.playing)
		if iteration % 25 == 0 or iteration == cycles - 1:
			# Observe after deferred node deletion and mixer retirement, not while
			# hundreds of deliberately stopped playbacks are awaiting a mix pass.
			await create_timer(0.15).timeout
			await process_frame
			var nodes := int(Performance.get_monitor(Performance.OBJECT_NODE_COUNT))
			var objects := int(Performance.get_monitor(Performance.OBJECT_COUNT))
			if baseline_nodes < 0:
				baseline_nodes = nodes
				baseline_objects = objects
			assert(nodes <= baseline_nodes + 2, "Node count grew across replacements")
			assert(objects <= baseline_objects + 16, "Object count grew across replacements")
	var playback: WeakRef = weakref(avatar.model.get_meta("ssc_audio").clips.tone.playback)
	avatar.fallback()
	avatar.free()
	var deadline := Time.get_ticks_msec() + 2000
	while playback.get_ref() != null:
		assert(Time.get_ticks_msec() < deadline, "Final playback survived stress cleanup")
		await process_frame
	DirAccess.remove_absolute(path)
	print("PASS %s avatar/audio/Luau replacement cycles, immediate decoder release and bounded settled node/object counts" % cycles)
	quit()

func _process(_delta: float) -> bool:
	if Time.get_ticks_msec() - started > 120000:
		push_error("Lifecycle stress exceeded its two-minute budget")
		quit(2)
	return false
