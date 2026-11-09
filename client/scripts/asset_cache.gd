extends RefCounted
## Completed ZIPs and dead-process-owned partials in the dedicated cache only.
## Import pins and recency stay in memory: no per-access metadata SSD writes.
static var active: Dictionary = {}
static var pins: Dictionary = {}
static var recent: Dictionary = {}
static var sequence := 0

static func valid_hash(value: String) -> bool:
	if value.length() != 64:
		return false
	for byte in value.to_ascii_buffer():
		if not ((byte >= 48 and byte <= 57) or (byte >= 97 and byte <= 102)):
			return false
	return true

static func managed(name: String) -> bool:
	return name.ends_with(".zip") and valid_hash(name.trim_suffix(".zip"))

static func partial_path(directory: String, hash: String) -> String:
	if not valid_hash(hash):
		return ""
	var nonce := Crypto.new().generate_random_bytes(16).hex_encode()
	if nonce.length() != 32:
		return ""
	return directory.path_join("%s.zip.part-%s-%s" % [hash, OS.get_process_id(), nonce])

static func partial_owner(name: String) -> int:
	var parts := name.split(".zip.part-", true)
	if parts.size() != 2 or not valid_hash(parts[0]):
		return 0
	var suffix := parts[1].split("-", true)
	if suffix.size() != 2 or suffix[1].length() != 32 or not valid_hash(suffix[1] + suffix[1]) or suffix[0].length() > 10 or not suffix[0].is_valid_int():
		return 0
	var pid := int(suffix[0])
	return pid if pid > 0 and pid <= 2147483647 and str(pid) == suffix[0] else 0

static func touch(path: String) -> void:
	if managed(path.get_file()):
		sequence += 1
		recent[path] = sequence

static func acquire(path: String) -> bool:
	if managed(path.get_file()):
		if not active.has(path):
			if not ClassDB.class_exists("CacheLease"):
				return false
			var pin = ClassDB.instantiate("CacheLease")
			if not pin.pin(ProjectSettings.globalize_path(path)).is_empty():
				return false
			pins[path] = pin
		active[path] = int(active.get(path, 0)) + 1
		touch(path)
		return true
	return false

static func release(path: String) -> void:
	if active.has(path):
		active[path] = int(active[path]) - 1
		if int(active[path]) <= 0:
			active.erase(path)
			pins.erase(path)

static func remove_completed(path: String) -> int:
	if not managed(path.get_file()) or not ClassDB.class_exists("CacheLease"):
		return 2
	var lease = ClassDB.instantiate("CacheLease")
	var result := int(lease.remove_completed(ProjectSettings.globalize_path(path)))
	if result == 0:
		recent.erase(path)
	return result

static func _older(a: Dictionary, b: Dictionary) -> bool:
	if a.recent != b.recent: return a.recent < b.recent
	if a.modified != b.modified: return a.modified < b.modified
	return str(a.path) < str(b.path)

static func _evict_oldest(candidates: Array[Dictionary], result: Dictionary) -> void:
	candidates.sort_custom(_older)
	var candidate: Dictionary = candidates.pop_front()
	var removed := remove_completed(str(candidate.path))
	if removed == 0:
		result.bytes -= int(candidate.bytes)
		result.files -= 1
		result.removed += 1
		recent.erase(candidate.path)
	elif removed != 1:
		result.errors += 1

static func trim(directory: String = "user://cache", max_bytes: int = 1073741824, max_files: int = 128) -> Dictionary:
	max_bytes = clampi(max_bytes, 0, 1073741824)
	max_files = clampi(max_files, 0, 128)
	var result := {"bytes":0,"files":0,"removed":0,"partials_removed":0,"errors":0,"over_budget":false}
	var dir := DirAccess.open(directory)
	if not dir:
		result.errors = 1
		return result
	var candidates: Array[Dictionary] = []
	# Godot's is_process_running() only recognizes children on Unix. The Nix
	# client instead uses Linux procfs; without it, preserve partials conservatively.
	var can_check_owners := OS.get_name() == "Linux" and DirAccess.dir_exists_absolute("/proc/%s" % OS.get_process_id())
	dir.list_dir_begin()
	var name := dir.get_next()
	while not name.is_empty():
		if not dir.current_is_dir() and not dir.is_link(name):
			var owner := partial_owner(name)
			if owner > 0 and can_check_owners and not DirAccess.dir_exists_absolute("/proc/%s" % owner):
				if DirAccess.remove_absolute(directory.path_join(name)) == OK:
					result.partials_removed += 1
				else:
					result.errors += 1
		if managed(name) and not dir.current_is_dir() and not dir.is_link(name):
			result.files += 1
			var path := directory.path_join(name)
			var file := FileAccess.open(path, FileAccess.READ)
			if file:
				var bytes := file.get_length()
				file.close()
				result.bytes += bytes
				if not active.has(path):
					candidates.append({"path":path,"bytes":bytes,"recent":int(recent.get(path,0)),"modified":FileAccess.get_modified_time(path)})
					# Keep at most 128 candidates even when opening a legacy cache
					# containing many files. An item older than 128 already seen
					# items cannot be among the globally newest 128.
					if candidates.size() > max_files:
						_evict_oldest(candidates, result)
			else:
				result.errors += 1
		name = dir.get_next()
	dir.list_dir_end()
	while not candidates.is_empty() and (int(result.bytes) > max_bytes or int(result.files) > max_files):
		_evict_oldest(candidates, result)
	for path in recent.keys():
		if str(path).get_base_dir() == directory and not FileAccess.file_exists(str(path)):
			recent.erase(path)
	result.over_budget = int(result.bytes) > max_bytes or int(result.files) > max_files
	return result
