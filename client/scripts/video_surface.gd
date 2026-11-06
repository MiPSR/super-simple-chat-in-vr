extends Node3D
## Decode is native/off-thread; textures and positional audio remain ordinary Godot.
signal failed(message: String)
const WIDTH := 640
const HEIGHT := 360
var decoder: RefCounted
var texture: ImageTexture
var player: AudioStreamPlayer3D
var playback: AudioStreamGeneratorPlayback
var source := ""
var generation := -1
var target := 0.0
var received := 0.0
var playing := false
var rate := 1.0
var correction_elapsed := 0.0
var pending := PackedVector2Array()
var decoded_frames := 0
var mixed_frames := 0
var broken := false

func setup(spec: Dictionary) -> void:
	var p: Array = spec.position
	position = Vector3(float(p[0]), float(p[1]), float(p[2]))
	var mesh := MeshInstance3D.new()
	var quad := QuadMesh.new()
	quad.size = Vector2(float(spec.size[0]), float(spec.size[1]))
	mesh.mesh = quad
	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.cull_mode = BaseMaterial3D.CULL_DISABLED
	texture = ImageTexture.create_from_image(Image.create(WIDTH, HEIGHT, false, Image.FORMAT_RGBA8))
	material.albedo_texture = texture
	mesh.material_override = material
	add_child(mesh)
	player = AudioStreamPlayer3D.new()
	var stream := AudioStreamGenerator.new()
	stream.mix_rate = 48000
	stream.buffer_length = 0.15
	player.stream = stream
	player.max_distance = 30
	add_child(player)
	decoder = ClassDB.instantiate("VideoDecoder") if ClassDB.class_exists("VideoDecoder") else null
	if not decoder:
		_fail("VideoDecoder native extension is unavailable")

func update_timeline(state: Dictionary) -> void:
	var was_playing := playing
	var previous_rate := rate
	target = float(state.get("position", 0))
	received = Time.get_ticks_msec() / 1000.0
	playing = bool(state.get("playing", false))
	rate = float(state.get("rate", 1))
	if not decoder:
		return
	var next := str(state.get("source", ""))
	var next_generation := int(state.get("generation", 0))
	var changed := next != source or next_generation != generation
	if changed:
		broken = false
		source = next
		generation = next_generation
		pending.clear()
		player.stop()
		playback = null
		if source.is_empty():
			decoder.close()
			return
		var error: String = decoder.open_url(source, WIDTH, HEIGHT)
		if not error.is_empty():
			_fail(error)
			return
		decoder.seek(target)
		player.play()
		playback = player.get_stream_playback()
	if changed or was_playing != playing:
		decoder.pause(not playing)
	if changed or previous_rate != rate:
		decoder.speed(rate)
	player.stream_paused = not playing

func _process(delta: float) -> void:
	if not decoder or source.is_empty() or broken:
		return
	var error: String = decoder.error()
	if not error.is_empty():
		_fail("Video playback failed: " + error)
		return
	var pixels: PackedByteArray = decoder.frame()
	if pixels.size() == WIDTH * HEIGHT * 4:
		decoded_frames += 1
		texture.update(Image.create_from_data(WIDTH, HEIGHT, false, Image.FORMAT_RGBA8, pixels))
	if playback and playing:
		if pending.is_empty():
			pending = decoder.audio()
		var count := mini(playback.get_frames_available(), pending.size())
		if count > 0:
			playback.push_buffer(pending.slice(0, count))
			mixed_frames += count
			pending = pending.slice(count)
	correction_elapsed += delta
	if correction_elapsed >= 1.0:
		correction_elapsed = 0
		var expected := target + (Time.get_ticks_msec() / 1000.0 - received) * rate if playing else target
		var drift := expected - float(decoder.position())
		if absf(drift) > 0.75:
			decoder.seek(expected)
			pending.clear()
			# Godot forbids clearing a generator while its playback is active.
			player.stop()
			player.play()
			playback = player.get_stream_playback()
			player.stream_paused = not playing
		elif playing:
			decoder.speed(clampf(rate + drift * 0.05, 0.25, 2.0))

func _fail(message: String) -> void:
	broken = true
	player.stop()
	playback = null
	pending.clear()
	if decoder:
		decoder.close()
	failed.emit(message)

func close() -> void:
	broken = true
	if player:
		player.stop()
	playback = null
	pending.clear()
	if decoder:
		decoder.close()
	decoder = null

func _exit_tree() -> void:
	close()
