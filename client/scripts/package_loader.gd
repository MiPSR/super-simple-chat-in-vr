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

func _ready() -> void:
	request = HTTPRequest.new()
	request.body_size_limit = 1_000_000_000
	request.timeout = 120
	add_child(request)
	request.request_completed.connect(_downloaded)
	DirAccess.make_dir_recursive_absolute(cache_directory)
	_trim_cache()

func load_world(snapshot: Dictionary) -> void:
	_load_asset(str(snapshot.world), str(snapshot.version), snapshot.manifest)

func load_avatar(id: String, version: String) -> void:
	request.body_size_limit = 100_000_000
	_load_asset(id, version, {})

func _load_asset(id: String, version: String, metadata: Dictionary) -> void:
	if not Cache.valid_hash(version):
		failed.emit("Invalid immutable package hash")
		return
	if worker or request.get_http_client_status() != HTTPClient.STATUS_DISCONNECTED:
		failed.emit("Another package is loading; wait before switching worlds.")
		return
	asset_id = id
	expected = version
	manifest = metadata
	staging = cache_directory.path_join(expected + ".zip")
	if FileAccess.file_exists(staging):
		_start_import()
		return
	request.download_file = Cache.partial_path(cache_directory, expected)
	if request.download_file.is_empty():
		failed.emit("Could not allocate a unique package download filename")
		return
	var error := request.request("%s/api/v1/assets/%s/download?version=%s" % [base_url, asset_id, expected], PackedStringArray(["Authorization: Bearer " + credentials]))
	if error != OK:
		DirAccess.remove_absolute(request.download_file)
		failed.emit("Could not start package download: %s" % error_string(error))

func _downloaded(result: int, code: int, _headers: PackedStringArray, _body: PackedByteArray) -> void:
	if result != HTTPRequest.RESULT_SUCCESS or code != 200:
		DirAccess.remove_absolute(request.download_file)
		failed.emit("Package download failed (%s / HTTP %s)" % [result, code])
		return
	if DirAccess.rename_absolute(request.download_file, staging) != OK:
		DirAccess.remove_absolute(request.download_file)
		failed.emit("Could not publish package cache file")
		return
	_start_import()

func _trim_cache() -> void:
	var summary := Cache.trim(cache_directory)
	if summary.over_budget or int(summary.errors) > 0:
		cache_notice.emit("Asset cache eviction deferred: active imports or filesystem errors")

func _start_import() -> void:
	pinned_cache_path = staging
	Cache.acquire(pinned_cache_path)
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
						result.prefabs[id] = {"document":prefab_doc, "state":prefab_state}
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
		if result.has("error"):
			DirAccess.remove_absolute(staging)
			failed.emit(result.error)
		else:
			if result.has("manifest"):
				manifest = result.manifest
			prefabs.clear()
			for id in result.get("prefabs", {}):
				var prefab = result.prefabs[id]
				var node: Node3D = prefab.document.generate_scene(prefab.state)
				if node:
					Materials.apply(node, quality)
					var scene := PackedScene.new()
					if scene.pack(node) == OK:
						prefabs[id] = scene
					node.free()
			# GPU upload/scene creation remains on the main thread; large scenes can stall.
			var root: Node3D = result.document.generate_scene(result.state)
			if root:
				Materials.apply(root, quality)
				loaded.emit(root)
			else:
				failed.emit("GLB scene construction failed")
		# Loaded listeners synchronously read Luau/Opus before this pin retires.
		Cache.release(imported_path)
		_trim_cache()

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
