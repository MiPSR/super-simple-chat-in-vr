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
	assert(Cache.remove_completed(path) == 0)
	assert(DirAccess.remove_absolute(directory.path_join(".ssc-cache.lock")) == OK)
	assert(DirAccess.remove_absolute(directory) == OK)
	print("PASS cancelled loader generations ignore stale HTTP callbacks/results and release import pins without publishing old scenes")
