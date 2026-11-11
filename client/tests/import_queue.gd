extends RefCounted
const Queue = preload("res://scripts/import_queue.gd")
const Loader = preload("res://scripts/package_loader.gd")
const Cache = preload("res://scripts/asset_cache.gd")

static func run() -> void:
	var first := Node.new()
	var second := Node.new()
	var third := Node.new()
	assert(Queue.request(first))
	assert(not Queue.request(second) and not Queue.request(third))
	Queue.release(first)
	assert(not Queue.request(third)) # Never jump ahead of an earlier waiter.
	assert(Queue.request(second))
	Queue.release(second)
	assert(Queue.request(third))
	third.free() # A dead owner cannot retain admission.
	assert(Queue.request(first))
	var directory := "user://import-queue-%s-%s" % [OS.get_process_id(),Time.get_ticks_usec()]
	assert(DirAccess.make_dir_recursive_absolute(directory) == OK)
	var path := directory.path_join("a".repeat(64) + ".zip")
	var file := FileAccess.open(path,FileAccess.WRITE)
	file.store_buffer(PackedByteArray([1,2,3]))
	file.close()
	var loader := Loader.new()
	loader.cache_directory = directory
	Engine.get_main_loop().root.add_child(loader)
	loader.set_process(false)
	loader.staging = path
	loader.expected = "a".repeat(64)
	loader._start_import()
	assert(loader.import_waiting and loader.worker == null and Cache.active.has(path))
	loader.cancel()
	assert(not loader.import_waiting and not Cache.active.has(path))
	assert(not Queue.waiters.any(func(reference: WeakRef): return reference.get_ref() == loader))
	loader.free()
	Queue.release(first)
	first.free()
	second.free()
	assert(Cache.remove_completed(path) == 0)
	assert(DirAccess.remove_absolute(directory.path_join(".ssc-cache.lock")) == OK)
	assert(DirAccess.remove_absolute(directory) == OK)
	print("PASS serialized FIFO import admission, dead-owner recovery, queued-cache pins and cancellation cleanup")
