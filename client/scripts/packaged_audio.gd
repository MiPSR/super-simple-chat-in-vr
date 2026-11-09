extends Node3D
## Memory-only Ogg Opus decode into native positional Godot generators.
signal failed(message: String)
var clips: Dictionary = {}
var muted := false

func set_muted(value: bool) -> void:
	muted = value
	for state in clips.values():
		state.player.volume_linear = 0.0 if muted else float(state.volume) * float(state.player.get_meta("ssc_occlusion_gain",1.0))

func load_package(path: String, declarations: Array) -> void:
	clear()
	if declarations.is_empty():
		return
	if not ClassDB.class_exists("PackagedOpus"):
		failed.emit("Packaged Opus extension unavailable")
		return
	var zip := ZIPReader.new()
	if zip.open(path) != OK:
		failed.emit("Could not open packaged audio")
		return
	for spec in declarations:
		var decoder = ClassDB.instantiate("PackagedOpus")
		var error: String = decoder.open_clip(zip.read_file(str(spec.path)))
		if not error.is_empty():
			failed.emit(error)
			continue
		var player := AudioStreamPlayer3D.new()
		var stream := AudioStreamGenerator.new()
		stream.mix_rate = 48000
		stream.buffer_length = 0.15
		player.stream = stream
		player.max_distance = 30
		player.volume_linear = 0.0 if muted else float(spec.get("volume", 1))
		var p: Array = spec.get("position", [0,0,0])
		player.position = Vector3(float(p[0]), float(p[1]), float(p[2]))
		add_child(player)
		clips[str(spec.id)] = {"decoder":decoder,"player":player,"playback":null,"pending":PackedVector2Array(),"loop":bool(spec.get("loop", false)),"playing":false,"eof":false,"capacity":0,"broken":false,"volume":float(spec.get("volume",1))}
		if bool(spec.get("autoplay", false)):
			play_sound(str(spec.id))
	zip.close()

func play_sound(id: String) -> void:
	if not clips.has(id) or bool(clips[id].broken):
		return
	stop_sound(id)
	var state: Dictionary = clips[id]
	state.player.play()
	state.playback = state.player.get_stream_playback()
	state.capacity = state.playback.get_frames_available()
	state.playing = true

func stop_sound(id: String) -> void:
	if not clips.has(id):
		return
	var state: Dictionary = clips[id]
	state.player.stop()
	state.playback = null
	state.pending = PackedVector2Array()
	state.playing = false
	state.eof = false
	state.decoder.seek(0)

func seek_sound(id: String, seconds: float) -> void:
	if not clips.has(id):
		return
	var state: Dictionary = clips[id]
	var was_playing := bool(state.playing)
	stop_sound(id)
	var error: String = state.decoder.seek(seconds)
	if not error.is_empty():
		failed.emit(error)
		return
	if was_playing:
		state.player.play()
		state.playback = state.player.get_stream_playback()
		state.capacity = state.playback.get_frames_available()
		state.playing = true

func _process(_delta: float) -> void:
	for id in clips:
		var state: Dictionary = clips[id]
		if not state.playing:
			continue
		if state.eof:
			if state.playback.get_frames_available() >= int(state.capacity):
				stop_sound(str(id))
			continue
		if state.pending.is_empty():
			state.pending = state.decoder.read_pcm(4800)
			var error: String = state.decoder.error()
			if not error.is_empty():
				stop_sound(str(id))
				state.broken = true
				state.decoder.close()
				failed.emit(error)
				continue
			if state.pending.is_empty():
				if state.loop:
					state.decoder.seek(0)
				else:
					# Stop only after already queued PCM drains, not a guessed delay.
					state.eof = true
				continue
		var count := mini(state.playback.get_frames_available(), state.pending.size())
		if count > 0:
			state.playback.push_buffer(state.pending.slice(0, count))
			state.pending = state.pending.slice(count)

func environment(acoustics: Node) -> void:
	for state in clips.values():
		state.player.bus = acoustics.route(state.player.global_position)
		state.player.volume_linear = 0.0 if muted else float(state.volume) * acoustics.source_gain(state.player)

func clear() -> void:
	for id in clips:
		stop_sound(str(id))
		clips[id].decoder.close()
		clips[id].player.queue_free()
	clips.clear()

func _exit_tree() -> void:
	clear()
