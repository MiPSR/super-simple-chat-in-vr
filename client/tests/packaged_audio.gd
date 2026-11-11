extends SceneTree
const Audio = preload("res://scripts/packaged_audio.gd")
const Creator = preload("res://scripts/creator_runtime.gd")
const Acoustics = preload("res://scripts/acoustics.gd")
const Avatar = preload("res://scripts/avatar.gd")
var started := 0

func _initialize() -> void:
	started = Time.get_ticks_msec()
	call_deferred("_run")

func _run() -> void:
	var path := "user://opus-test-%s.zip" % Time.get_ticks_usec()
	var zip := ZIPPacker.new()
	assert(zip.open(path) == OK)
	zip.start_file("audio/bell.opus")
	zip.write_file(FileAccess.get_file_as_bytes(OS.get_environment("SSC_TEST_OPUS")))
	zip.close_file()
	zip.start_file("audio/invalid.opus")
	zip.write_file("RIFFnot-opus".to_utf8_buffer())
	zip.close_file()
	zip.start_file("scripts/audio.luau")
	zip.write_file('world.on("action", function(data) if data.name == "play" then world.play_sound("bell") elseif data.name == "seek" then world.seek_sound("bell", 0.1) else world.stop_sound("bell") end end)'.to_utf8_buffer())
	zip.close_file()
	zip.close()
	var scope := Node3D.new()
	root.add_child(scope)
	var audio := Audio.new()
	scope.add_child(audio)
	scope.set_meta("ssc_audio", audio)
	audio.failed.connect(func(message: String): push_error(message); quit(2))
	audio.load_package(path, [{"id":"bell","path":"audio/bell.opus","position":[1,2,3],"loop":true}])
	assert(audio.clips.size() == 1)
	assert(audio.clips.bell.player.position.is_equal_approx(Vector3(1,2,3)))
	assert(is_equal_approx(audio.clips.bell.decoder.duration(), 0.3))
	var creator := Creator.new()
	scope.add_child(creator)
	creator.set_process(false) # Commands below deliberately assert synchronous results.
	creator.script_error.connect(func(message: String): push_error(message); quit(2))
	creator.load_package(path, {"scripts":[{"path":"scripts/audio.luau","side":"client","attachment":"world","capabilities":["audio"]}]}, scope)
	var playbacks: Array[WeakRef] = []
	audio.set_muted(true)
	creator.dispatch("action", {"name":"play"})
	playbacks.append(weakref(audio.clips.bell.playback))
	assert(is_zero_approx(audio.clips.bell.player.volume_linear))
	await create_timer(0.08).timeout
	assert(audio.clips.bell.playing and audio.clips.bell.decoder.position() > 0)
	audio.set_muted(false)
	assert(is_equal_approx(audio.clips.bell.player.volume_db, 0))
	creator.dispatch("action", {"name":"seek"})
	playbacks.append(weakref(audio.clips.bell.playback))
	assert(is_equal_approx(audio.clips.bell.decoder.position(), 0.1))
	creator.dispatch("action", {"name":"stop"})
	assert(not audio.clips.bell.playing and audio.clips.bell.playback == null)
	creator.dispatch("action", {"name":"play"})
	playbacks.append(weakref(audio.clips.bell.playback))
	await create_timer(0.5).timeout
	assert(audio.clips.bell.playing and audio.clips.bell.decoder.position() <= 0.3)
	audio.clips.bell.loop = false
	audio.seek_sound("bell", 0.28)
	playbacks.append(weakref(audio.clips.bell.playback))
	var end_deadline := Time.get_ticks_msec() + 2000
	while audio.clips.bell.playing:
		assert(Time.get_ticks_msec() < end_deadline, "Completed sound did not drain")
		await process_frame
	creator.dispatch("action", {"name":"play"})
	playbacks.append(weakref(audio.clips.bell.playback))
	var acoustics := Acoustics.new()
	scope.add_child(acoustics)
	acoustics.configure([{"id":"test","position":[1,2,3],"size":[2,2,2],"effects":{"reverb":0.2}}], 1)
	audio.environment(acoustics)
	assert(audio.clips.bell.player.bus == acoustics.route(Vector3(1,2,3)))
	creator.close()
	scope.free()
	var deadline := Time.get_ticks_msec() + 2000
	while playbacks.any(func(reference: WeakRef) -> bool: return reference.get_ref() != null):
		assert(Time.get_ticks_msec() < deadline, "Packaged audio playback survived cleanup")
		await process_frame
	var avatar := Avatar.new()
	root.add_child(avatar)
	var notices: Array[String] = []
	avatar.runtime_status.connect(func(message: String): notices.append(message))
	avatar.set_audio_allowed(false)
	avatar.set_model(Node3D.new(), path, {"sounds":[{"id":"bell","path":"audio/bell.opus"}],"scripts":[]})
	var old_audio: Node = avatar.model.get_meta("ssc_audio")
	assert(is_zero_approx(old_audio.clips.bell.player.volume_linear))
	var old_decoder: RefCounted = old_audio.clips.bell.decoder
	assert(is_equal_approx(old_decoder.duration(), 0.3))
	avatar.set_model(Node3D.new(), path, {"sounds":[{"id":"bad","path":"audio/invalid.opus"}],"scripts":[]})
	assert(is_zero_approx(old_decoder.duration()))
	assert(not notices.is_empty())
	old_decoder = null
	avatar.free()
	DirAccess.remove_absolute(path)
	var invalid = ClassDB.instantiate("PackagedOpus")
	assert(not invalid.open_clip("RIFFnot-opus".to_utf8_buffer()).is_empty())
	invalid.close()
	invalid = null
	print("PASS packaged Ogg Opus, client Luau play/stop/seek, looping, spatial effects and cleanup")
	quit()

func _process(_delta: float) -> bool:
	if Time.get_ticks_msec() - started > 5000:
		push_error("Packaged audio test timed out")
		quit(2)
	return false
