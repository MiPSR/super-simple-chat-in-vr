extends Node
## Only detached JSON and capability-filtered visual commands cross the VM boundary.
signal script_error(message: String)
var scripts: Array[Dictionary] = []
var scope: Node
var disabled := false
var scene: RefCounted
var world_queries := false
var scene_error := ""
var pending_events: Array[Dictionary] = []
var event_overflow := false

func queue_world_events(events: Array) -> void:
	if scripts.is_empty():
		return
	for event in events:
		if not event is Dictionary or event.get("event") not in ["grab", "lease", "release", "damage", "destroy"]:
			continue
		if pending_events.size() >= 128:
			event_overflow = true
			continue
		pending_events.append(event.duplicate(true))

func _dispatch_world_events() -> void:
	if event_overflow:
		script_error.emit("Client creator event queue overflow; newest notifications dropped")
		event_overflow = false
	# Snapshot the batch: callbacks cannot recursively drain newly queued events.
	var count := mini(pending_events.size(), 8)
	var start := Time.get_ticks_usec()
	for index in count:
		if index > 0 and Time.get_ticks_usec() - start >= 5000:
			break
		var event: Dictionary = pending_events.pop_front()
		dispatch(str(event.event), event)

func _ensure_scene() -> bool:
	if not scene and ClassDB.class_exists("CreatorScene"):
		scene = ClassDB.instantiate("CreatorScene")
	return scene != null

func _scene_result(error: String) -> void:
	if not error.is_empty():
		scene.clear()
		if error != scene_error:
			script_error.emit(error)
	scene_error = error

func update_objects(objects: Dictionary, tick: int) -> void:
	if world_queries and _ensure_scene():
		_scene_result(scene.update_objects(JSON.stringify(objects).to_utf8_buffer(), tick))

func update_transforms(objects: Array, tick: int) -> void:
	if world_queries and _ensure_scene():
		_scene_result(scene.update_transforms(JSON.stringify(objects).to_utf8_buffer(), tick))

func update_state(variables: Dictionary, tick: int) -> void:
	if world_queries and _ensure_scene():
		_scene_result(scene.update_state(JSON.stringify(variables).to_utf8_buffer(), tick))

func load_package(path: String, manifest: Dictionary, root: Node, keep_world_snapshot: bool = false) -> void:
	close(not keep_world_snapshot)
	scope = root
	if not manifest.get("scripts", []).any(func(s): return s.get("side") == "client"):
		if scene:
			scene.clear()
		return
	if not ClassDB.class_exists("CreatorVm") or not _ensure_scene():
		if manifest.get("scripts", []).any(func(s): return s.get("side") == "client"):
			script_error.emit("Client Luau extension is unavailable; use the Nix-packaged client")
		return
	var zip := ZIPReader.new()
	if zip.open(path) != OK:
		return
	for declaration in manifest.get("scripts", []):
		if declaration.side != "client":
			continue
		var source := zip.read_file(str(declaration.path)).get_string_from_utf8()
		var vm = ClassDB.instantiate("CreatorVm")
		vm.bind_scene(scene)
		var error: String = vm.load_script(source, PackedStringArray(declaration.capabilities))
		if error.is_empty():
			scripts.append({"vm":vm,"attachment":str(declaration.attachment),"failed":false})
		else:
			script_error.emit(error)
	zip.close()

func dispatch(event: String, data: Dictionary) -> void:
	if not is_instance_valid(scope):
		return
	for script in scripts:
		if script.failed:
			continue
		var response = JSON.parse_string(script.vm.dispatch(event, JSON.stringify(data)))
		if not response is Dictionary:
			continue
		if response.has("error"):
			script.failed = true
			script_error.emit(str(response.error))
			continue
		for command in response.get("commands", []):
			_apply(command, script.attachment)

func _apply(command: Dictionary, attachment: String) -> void:
	# Exact node names, not paths: neither '..' nor an absolute path can escape scope.
	var target := str(command.get("target", ""))
	if command.action == "radio" and scope.has_meta("ssc_radio"):
		var radio: Node = scope.get_meta("ssc_radio")
		radio.transmit(target, str(command.mode), str(command.data))
	elif command.action in ["play_sound", "stop_sound", "seek_sound"] and scope.has_meta("ssc_audio"):
		var audio: Node = scope.get_meta("ssc_audio")
		match command.action:
			"play_sound": audio.play_sound(target)
			"stop_sound": audio.stop_sound(target)
			"seek_sound": audio.seek_sound(target, float(command.value))
	elif command.action == "visible":
		for node in scope.find_children("*", "Node3D", true, false):
			if str(node.name) == target and (attachment in ["world", "avatar"] or str(node.name) == attachment or scope.name == attachment):
				node.visible = bool(command.value)
	elif command.action in ["bone_rotation", "reset_bone"]:
		if scope.has_method("creator_bone") and (attachment == "avatar" or attachment == target):
			var error: String = scope.creator_bone(command)
			if not error.is_empty():
				script_error.emit(error)
	elif command.action == "spring_gravity":
		if attachment == "avatar" and scope.has_method("creator_spring_gravity"):
			var error: String = scope.creator_spring_gravity(float(command.value))
			if not error.is_empty():
				script_error.emit(error)
	elif scope.has_method("creator_action"):
		scope.creator_action(str(command.action), target, float(command.get("value", 1)))

func _process(delta: float) -> void:
	_dispatch_world_events()
	var event := {"dt":delta}
	if is_instance_valid(scope) and scope.has_method("creator_tracking"):
		event.tracking = scope.creator_tracking()
	dispatch("tick", event)

func close(clear_snapshot: bool = true) -> void:
	for script in scripts:
		script.vm.close()
	scripts.clear()
	pending_events.clear()
	event_overflow = false
	scope = null
	if clear_snapshot:
		world_queries = false
		scene_error = ""
		if scene:
			scene.clear()

func _exit_tree() -> void:
	close()
