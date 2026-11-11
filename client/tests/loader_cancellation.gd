extends RefCounted
const Loader = preload("res://scripts/package_loader.gd")
const Cache = preload("res://scripts/asset_cache.gd")

class Document extends RefCounted:
	var calls := 0
	func generate_scene(_state: Variant) -> Node3D:
		calls += 1
		return Node3D.new()

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var directory := "user://cancelled-loader-%s-%s" % [OS.get_process_id(),Time.get_ticks_usec()]
	assert(DirAccess.make_dir_recursive_absolute(directory) == OK)
	var path := directory.path_join("a".repeat(64) + ".zip")
	var file := FileAccess.open(path,FileAccess.WRITE)
	file.store_buffer(PackedByteArray([1,2,3]))
	file.close()
	var loader := Loader.new()
	loader.cache_directory = directory
	tree.root.add_child(loader)
	loader.set_process(false)
	var counts := {"loaded":0,"errors":0}
	loader.loaded.connect(func(root: Node3D): counts.loaded += 1; root.free())
	loader.failed.connect(func(_error: String): counts.errors += 1)
	var document := Document.new()
	loader.pending_result = {"document":document,"state":null,"prefabs":{}}
	loader.staging = path
	loader.pinned_cache_path = path
	assert(Cache.acquire(path))
	loader.worker = Thread.new()
	assert(loader.worker.start(func(): return 0) == OK)
	var old_generation := loader.request_generation
	loader.cancel()
	loader._downloaded(0,200,PackedStringArray(),PackedByteArray(),old_generation)
	var deadline := Time.get_ticks_msec() + 2000
	while loader.worker.is_alive() and Time.get_ticks_msec() < deadline:
		await tree.process_frame
	assert(not loader.worker.is_alive())
	loader._process(0)
	assert(document.calls == 0 and counts.loaded == 0 and counts.errors == 0)
	assert(not Cache.active.has(path) and FileAccess.file_exists(path))
	loader.free()
	# Retirement must return while the worker is blocked, then discard its
	# eventual result and retire the owner asynchronously rather than joining.
	loader = Loader.new()
	loader.cache_directory = directory
	tree.root.add_child(loader)
	loader.set_process(false)
	loader.pending_result = {"document":document,"state":null,"prefabs":{}}
	loader.staging = path
	loader.pinned_cache_path = path
	assert(Cache.acquire(path))
	var gate := Semaphore.new()
	loader.worker = Thread.new()
	assert(loader.worker.start(func(): gate.wait()) == OK)
	var retired: WeakRef = weakref(loader)
	loader.retire()
	assert(loader.worker.is_alive() and not loader.is_queued_for_deletion())
	gate.post()
	loader = null
	deadline = Time.get_ticks_msec() + 2000
	while retired.get_ref() != null and Time.get_ticks_msec() < deadline:
		await tree.process_frame
	assert(retired.get_ref() == null and not Cache.active.has(path) and document.calls == 0)
	# A failure listener may synchronously request another immutable package.
	# Keep its request queued until the old pin/queue slot has fully retired.
	var failed_path := directory.path_join("c".repeat(64) + ".zip")
	var replacement_path := directory.path_join("b".repeat(64) + ".zip")
	for destination in [failed_path,replacement_path]:
		file = FileAccess.open(destination,FileAccess.WRITE)
		file.store_buffer(PackedByteArray([1,2,3]))
		file.close()
	var replacement_loader := Loader.new()
	replacement_loader.cache_directory = directory
	tree.root.add_child(replacement_loader)
	replacement_loader.set_process(false)
	replacement_loader.pending_result = {"error":"fixture failure"}
	replacement_loader.staging = failed_path
	replacement_loader.pinned_cache_path = failed_path
	assert(Cache.acquire(failed_path))
	replacement_loader.failed.connect(func(_error: String):
		assert(replacement_loader.finishing_import and Cache.active.has(failed_path))
		replacement_loader.load_avatar("replacement","b".repeat(64))
		assert(replacement_loader.worker == null and not replacement_loader.queued_load.is_empty())
	,CONNECT_ONE_SHOT)
	replacement_loader.worker = Thread.new()
	assert(replacement_loader.worker.start(func(): return 0) == OK)
	while replacement_loader.worker.is_alive():
		await tree.process_frame
	replacement_loader._process(0)
	assert(not FileAccess.file_exists(failed_path) and not Cache.active.has(failed_path))
	assert(FileAccess.file_exists(replacement_path) and Cache.active.has(replacement_path))
	assert(replacement_loader.worker != null and not replacement_loader.finishing_import)
	replacement_loader.cancel()
	while replacement_loader.worker.is_alive():
		await tree.process_frame
	replacement_loader._process(0)
	replacement_loader.free()
	assert(Cache.remove_completed(replacement_path) == 0)
	assert(Cache.remove_completed(path) == 0)
	assert(DirAccess.remove_absolute(directory.path_join(".ssc-cache.lock")) == OK)
	assert(DirAccess.remove_absolute(directory) == OK)
	print("PASS cancelled/retired loader generations, stale callbacks, cache pins and reentrant replacement cleanup")
