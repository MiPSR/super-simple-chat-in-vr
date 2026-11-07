extends SceneTree
const Surface = preload("res://scripts/video_surface.gd")
var surface: Node3D
var failures: Array[String] = []
var playbacks: Dictionary = {}

func _initialize() -> void:
	call_deferred("_run")

func _process(_delta: float) -> bool:
	if is_instance_valid(surface) and surface.playback:
		playbacks[surface.playback.get_instance_id()] = weakref(surface.playback)
	return false

func _run() -> void:
	surface = Surface.new()
	root.add_child(surface)
	surface.failed.connect(func(message: String): failures.append(message))
	surface.setup({"position":[0,2,-3],"size":[3.2,1.8]})
	var urls: Array = JSON.parse_string(OS.get_environment("SSC_TEST_VIDEO_FAILURES"))
	var generation := 0
	for url in urls:
		generation += 1
		var before := failures.size()
		surface.update_timeline({"source":str(url),"position":0,"playing":true,"rate":1,"generation":generation})
		var deadline := Time.get_ticks_msec() + 8000
		while failures.size() == before:
			if Time.get_ticks_msec() >= deadline:
				push_error("Missing media failure message for " + str(url))
				quit(2)
				return
			await process_frame
		assert(not failures[-1].is_empty() and surface.broken)
		assert(not surface.player.playing and surface.playback == null and surface.pending.is_empty())
		# The same failed generation must stay stopped rather than reopen on each
		# authoritative state update and repeatedly emit the same user-facing error.
		surface.update_timeline({"source":str(url),"position":0,"playing":true,"rate":1,"generation":generation})
		await process_frame
		assert(failures.size() == before + 1 and surface.broken)
		generation += 1
		var old_frames: int = surface.decoded_frames
		var old_pcm: int = surface.mixed_frames
		surface.update_timeline({"source":OS.get_environment("SSC_TEST_VIDEO"),"position":0,"playing":true,"rate":1,"generation":generation})
		deadline = Time.get_ticks_msec() + 8000
		while surface.decoded_frames <= old_frames or surface.mixed_frames <= old_pcm:
			if Time.get_ticks_msec() >= deadline or failures.size() != before + 1:
				push_error("Media recovery failed after " + str(url) + ": " + str(failures))
				quit(2)
				return
			await process_frame
		assert(not surface.broken and surface.player.playing)
	surface.close()
	surface.close()
	surface.queue_free()
	await process_frame
	var cleanup_deadline := Time.get_ticks_msec() + 2000
	while playbacks.values().any(func(reference: WeakRef) -> bool: return reference.get_ref() != null):
		assert(Time.get_ticks_msec() < cleanup_deadline, "Failed/recovered media playback survived cleanup")
		await process_frame
	print("PASS removed/restricted/unsupported HTTP media errors, stopped failed generations, recovery decode/PCM and cleanup")
	quit()
