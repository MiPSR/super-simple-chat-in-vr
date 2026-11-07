extends RefCounted
## Only completed, content-addressed ZIPs in the dedicated cache are managed.
## Import pins and recency stay in memory: no per-access metadata SSD writes.
static var active: Dictionary = {}
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

static func touch(path: String) -> void:
	if managed(path.get_file()):
		sequence += 1
		recent[path] = sequence

static func acquire(path: String) -> void:
	if managed(path.get_file()):
		active[path] = int(active.get(path, 0)) + 1
		touch(path)

static func release(path: String) -> void:
	if active.has(path):
		active[path] = int(active[path]) - 1
		if int(active[path]) <= 0:
			active.erase(path)

static func _older(a: Dictionary, b: Dictionary) -> bool:
	if a.recent != b.recent: return a.recent < b.recent
	if a.modified != b.modified: return a.modified < b.modified
	return str(a.path) < str(b.path)

static func _evict_oldest(candidates: Array[Dictionary], result: Dictionary) -> void:
	candidates.sort_custom(_older)
	var candidate: Dictionary = candidates.pop_front()
	if DirAccess.remove_absolute(candidate.path) == OK:
		result.bytes -= int(candidate.bytes)
		result.files -= 1
		result.removed += 1
		recent.erase(candidate.path)
	else:
		result.errors += 1

static func trim(directory: String = "user://cache", max_bytes: int = 1073741824, max_files: int = 128) -> Dictionary:
	max_bytes = clampi(max_bytes, 0, 1073741824)
	max_files = clampi(max_files, 0, 128)
	var result := {"bytes":0,"files":0,"removed":0,"errors":0,"over_budget":false}
	var dir := DirAccess.open(directory)
	if not dir:
		result.errors = 1
		return result
	var candidates: Array[Dictionary] = []
	dir.list_dir_begin()
	var name := dir.get_next()
	while not name.is_empty():
		if not dir.current_is_dir() and not dir.is_link(name) and managed(name):
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
