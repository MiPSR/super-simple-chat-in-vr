extends SceneTree
const Surface = preload("res://scripts/video_surface.gd")
var surface: Node3D
var started := 0
var saw_frame := false
var phase := 0
var paused_position := 0.0
var saw_seek := false
var stable_since := 0
var stable_position := 0.0
var playbacks: Dictionary = {}

func _initialize() -> void:
	call_deferred("_start")

func _start() -> void:
	started = Time.get_ticks_msec()
	surface = Surface.new()
	root.add_child(surface)
	surface.failed.connect(func(message: String):
		push_error(message)
		quit(2)
	)
	surface.setup({"position":[0,2,-3], "size":[3.2,1.8]})
	surface.update_timeline({"source":OS.get_environment("SSC_TEST_VIDEO"), "position":0, "playing":true, "rate":1, "generation":1})

func _process(_delta: float) -> bool:
	if not is_instance_valid(surface) or phase == 3:
		return false
	if surface.playback and not playbacks.has(surface.playback.get_instance_id()):
		playbacks[surface.playback.get_instance_id()] = weakref(surface.playback)
	if surface.decoded_frames > 0 and surface.mixed_frames > 0 and float(surface.decoder.position()) > 0.1:
		saw_frame = true
	if saw_frame:
		if phase == 0:
			paused_position = float(surface.decoder.position())
			surface.update_timeline({"source":OS.get_environment("SSC_TEST_VIDEO"), "position":paused_position, "playing":false, "rate":1, "generation":1})
			phase = 1
		elif phase == 1:
			var current := float(surface.decoder.position())
			if absf(current - paused_position) > 0.25 or absf(current - stable_position) > 0.02:
				stable_since = Time.get_ticks_msec()
				stable_position = current
			elif Time.get_ticks_msec() - stable_since > 250:
				surface.decoder.seek(0.8)
				surface.update_timeline({"source":OS.get_environment("SSC_TEST_VIDEO"), "position":0.8, "playing":true, "rate":1, "generation":1})
				phase = 2
		elif phase == 2:
			var position := float(surface.decoder.position())
			if absf(position - 0.8) < 0.15:
				saw_seek = true
			if saw_seek and position > 0.9:
				surface.update_timeline({"source":"", "position":0, "playing":false, "rate":1, "generation":1})
				surface.queue_free()
				phase = 3
				call_deferred("_finish")
	if Time.get_ticks_msec() - started > 10000:
		push_error("Video surface test timed out in phase " + str(phase))
		quit(2)
	return false

func _finish() -> void:
	await process_frame
	# Seek/drift correction can create multiple mixer-owned playbacks. Observe
	# all of them retiring rather than assuming a fixed cleanup sleep is enough.
	var deadline := Time.get_ticks_msec() + 2000
	while playbacks.values().any(func(reference: WeakRef) -> bool: return reference.get_ref() != null):
		assert(Time.get_ticks_msec() < deadline, "Video playback survived cleanup")
		await process_frame
	print("PASS HTTP libmpv decode, Godot texture/positional PCM, pause/seek/resume and cleanup")
	quit()
