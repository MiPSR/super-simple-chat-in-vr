extends Node
signal loaded(root: Node3D)
signal failed(message: String)
signal cache_notice(message: String)
const Materials = preload("res://scripts/materials.gd")
const Cache = preload("res://scripts/asset_cache.gd")
var request: HTTPRequest
var credentials := ""
var base_url := ""
var expected := ""
var manifest: Dictionary
var quality := 1
var texture_reduction := 0
var worker: Thread
var pending_result: Dictionary = {}
var result_mutex := Mutex.new()
var staging := ""
var prefabs: Dictionary = {}
var asset_id := ""
var local_avatar := false
var pinned_cache_path := ""
var cache_directory := "user://cache"
var import_cancelled := false
var queued_load: Dictionary = {}
var request_generation := 0
var download_limit := 1_000_000_000

func cancel(clear_queued: bool = true) -> void:
	request_generation += 1
	if clear_queued:
		queued_load.clear()
	if request:
		request.cancel_request()
		if not request.download_file.is_empty():
			DirAccess.remove_absolute(request.download_file)
			request.download_file = ""
	if worker:
		# Never join a running GLTF importer from a focus/leave/UI callback.
		# Its completed result is discarded on the main thread before publication.
		import_cancelled = true

func _ready() -> void:
	_renew_request()
	DirAccess.make_dir_recursive_absolute(cache_directory)
	_trim_cache()

func _renew_request() -> void:
	request_generation += 1
	if request:
		request.cancel_request()
		request.queue_free()
	request = HTTPRequest.new()
	request.body_size_limit = download_limit
	request.timeout = 120
	add_child(request)
	request.request_completed.connect(_downloaded.bind(request_generation))

func load_world(snapshot: Dictionary) -> void:
	download_limit = 1_000_000_000
	_load_asset(str(snapshot.world), str(snapshot.version), snapshot.manifest)

func load_avatar(id: String, version: String) -> void:
	download_limit = 100_000_000
	_load_asset(id, version, {})

func _load_asset(id: String, version: String, metadata: Dictionary) -> void:
	if not Cache.valid_hash(version):
		failed.emit("Invalid immutable package hash")
		return
	if worker or request.get_http_client_status() != HTTPClient.STATUS_DISCONNECTED:
		cancel(false)
		if worker:
			queued_load = {"id":id,"version":version,"metadata":metadata.duplicate(true)}
			return
	asset_id = id
	expected = version
	manifest = metadata
	staging = cache_directory.path_join(expected + ".zip")
	if FileAccess.file_exists(staging):
		_start_import()
		return
	_renew_request()
	request.download_file = Cache.partial_path(cache_directory, expected)
	if request.download_file.is_empty():
		failed.emit("Could not allocate a unique package download filename")
		return
	var error := request.request("%s/api/v1/assets/%s/download?version=%s" % [base_url, asset_id, expected], PackedStringArray(["Authorization: Bearer " + credentials]))
	if error != OK:
		DirAccess.remove_absolute(request.download_file)
		failed.emit("Could not start package download: %s" % error_string(error))

func _downloaded(result: int, code: int, _headers: PackedStringArray, _body: PackedByteArray, generation: int) -> void:
	if generation != request_generation:
		return
	if result != HTTPRequest.RESULT_SUCCESS or code != 200:
		DirAccess.remove_absolute(request.download_file)
		failed.emit("Package download failed (%s / HTTP %s)" % [result, code])
		return
	if not ClassDB.class_exists("CacheLease"):
		DirAccess.remove_absolute(request.download_file)
		failed.emit("Cross-process cache locking extension unavailable")
		return
	# Publication is no-replace and stays shared-pinned until _start_import has
	# acquired its own lease. A parallel downloader never replaces a pinned ZIP.
	var publication = ClassDB.instantiate("CacheLease")
	var publication_error: String = publication.publish_download(ProjectSettings.globalize_path(request.download_file),ProjectSettings.globalize_path(staging))
	if not publication_error.is_empty():
		DirAccess.remove_absolute(request.download_file)
		failed.emit("Could not publish package cache file: " + publication_error)
		return
	_start_import()

func _trim_cache() -> void:
	var summary := Cache.trim(cache_directory)
	if summary.over_budget or int(summary.errors) > 0:
		cache_notice.emit("Asset cache eviction deferred: active imports or filesystem errors")

func _start_import() -> void:
	import_cancelled = false
	pinned_cache_path = staging
	if not Cache.acquire(pinned_cache_path):
		pinned_cache_path = ""
		failed.emit("Could not pin completed package for import; retry the load")
		return
	_trim_cache()
	worker = Thread.new()
	var error := worker.start(_import.bind(staging, expected, str(manifest.get("scene", "")), manifest.get("prefabs", {}).duplicate(true), local_avatar, texture_reduction))
	if error != OK:
		worker = null
		Cache.release(pinned_cache_path)
		pinned_cache_path = ""
		failed.emit("Could not start package importer")

func _import(path: String, hash: String, scene: String, definitions: Dictionary, first_person: bool, reduction: int = 0) -> void:
	var result: Dictionary = {}
	# Streaming hash avoids a second package-sized allocation.
	if FileAccess.get_sha256(path) != hash:
		result.error = "Package hash mismatch"
	else:
		var zip := ZIPReader.new()
		if zip.open(path) != OK:
			result.error = "Invalid cached ZIP"
		else:
			if scene.is_empty():
				var metadata = JSON.parse_string(zip.read_file("manifest.json").get_string_from_utf8())
				if metadata is Dictionary:
					scene = str(metadata.get("scene", ""))
					result.manifest = metadata
			var document := GLTFDocument.new()
			var state := GLTFState.new()
			state.set_additional_data(&"ssc/texture_reduction", clampi(reduction, 0, 3))
			state.set_additional_data(&"vrm/head_hiding_method", 1 if first_person else 0)
			var bytes := zip.read_file(scene)
			if document.append_from_buffer(bytes, "", state) != OK:
				result.error = "GLB import failed"
			else:
				result.document = document
				result.state = state
				result.prefabs = {}
				for id in definitions:
					var prefab_doc := GLTFDocument.new()
					var prefab_state := GLTFState.new()
					prefab_state.set_additional_data(&"ssc/texture_reduction", clampi(reduction, 0, 3))
					if prefab_doc.append_from_buffer(zip.read_file(str(definitions[id].scene)), "", prefab_state) == OK:
						result.prefabs[id] = {"document":prefab_doc, "state":prefab_state, "materials":definitions[id].get("materials",{})}
					else:
						result.error = "Packaged prefab import failed: " + str(id)
						break
			zip.close()
	result_mutex.lock()
	pending_result = result
	result_mutex.unlock()

func _process(_delta: float) -> void:
	if worker and not worker.is_alive():
		worker.wait_to_finish()
		worker = null
		var imported_path := pinned_cache_path
		pinned_cache_path = ""
		result_mutex.lock()
		var result := pending_result
		pending_result = {}
		result_mutex.unlock()
		if import_cancelled:
			pass # Inactive generations never create scenes, scripts or audio.
		elif result.has("error"):
			failed.emit(result.error)
		else:
			if result.has("manifest"):
				manifest = result.manifest
			prefabs.clear()
			var construction_error := ""
			for id in result.get("prefabs", {}):
				var prefab = result.prefabs[id]
				var node: Node3D = prefab.document.generate_scene(prefab.state)
				if node:
					preload("res://scripts/creator_animations.gd").normalize(node)
					Materials.prepare(node, quality)
					construction_error = preload("res://scripts/material_styles.gd").apply(node,prefab.materials)
					if not construction_error.is_empty():
						node.free()
						break
					var scene := PackedScene.new()
					var reservation := preload("res://scripts/render_budget.gd").reserve(node)
					if reservation.has("error"):
						construction_error = str(reservation.error)
						node.free()
						break
					if scene.pack(node) == OK:
						scene.set_meta("ssc_render_lease",reservation.lease)
						prefabs[id] = scene
					else:
						construction_error = "Could not pack prefab scene"
					node.free()
					if not construction_error.is_empty():
						break
				else:
					construction_error = "Prefab scene construction failed"
					break
			# GPU upload/scene creation remains on the main thread; large scenes can stall.
			var root: Node3D = result.document.generate_scene(result.state) if construction_error.is_empty() else null
			if root:
				preload("res://scripts/creator_animations.gd").normalize(root)
				Materials.prepare(root, quality)
				construction_error = preload("res://scripts/material_styles.gd").apply(root,manifest.get("materials",{}))
				if construction_error.is_empty():
					var reservation := preload("res://scripts/render_budget.gd").reserve(root)
					if reservation.has("error"):
						construction_error = str(reservation.error)
					else:
						root.set_meta("ssc_render_lease",reservation.lease)
				if construction_error.is_empty():
					loaded.emit(root)
				else:
					root.free()
					prefabs.clear()
					failed.emit(construction_error)
			else:
				prefabs.clear()
				failed.emit(construction_error if not construction_error.is_empty() else "GLB scene construction failed")
		# Loaded listeners synchronously read Luau/Opus before this pin retires.
		Cache.release(imported_path)
		if result.has("error") and not import_cancelled:
			Cache.remove_completed(staging)
		import_cancelled = false
		_trim_cache()
		if not queued_load.is_empty():
			var next := queued_load
			queued_load = {}
			_load_asset(str(next.id),str(next.version),next.metadata)

func _exit_tree() -> void:
	if request:
		request.cancel_request()
		if not request.download_file.is_empty():
			DirAccess.remove_absolute(request.download_file)
	if worker:
		worker.wait_to_finish()
	var was_pinned := not pinned_cache_path.is_empty()
	Cache.release(pinned_cache_path)
	pinned_cache_path = ""
	if was_pinned:
		Cache.trim(cache_directory)
