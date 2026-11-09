extends Node
const Visuals = preload("res://scripts/creator_visuals.gd")
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
var media_window := 0
var media_requests := 0
var media_overflow := false
var avatar_queries := false
static var runtimes: Array[WeakRef] = []
static var scheduler_frame := -1
static var scheduler_cursor := 0
var jobs: Array[Dictionary] = []
var job_bytes := 0
var tick_elapsed := 0.0
var tick_queued := false

func _enter_tree() -> void:
	runtimes.append(weakref(self))

static func advance_all(delta: float, frame: int, maximum_steps: int = 256, budget_usec: int = 4000) -> void:
	if frame == scheduler_frame:
		return
	scheduler_frame = frame
	var active: Array[Node] = []
	var retained: Array[WeakRef] = []
	for reference in runtimes:
		var runtime = reference.get_ref()
		if not is_instance_valid(runtime) or not runtime.is_inside_tree():
			continue
		retained.append(reference)
		if runtime.is_processing() and not runtime.disabled and not runtime.scripts.is_empty() and is_instance_valid(runtime.scope):
			active.append(runtime)
			runtime._prepare_frame(delta)
	runtimes = retained
	if active.is_empty():
		return
	var started := Time.get_ticks_usec()
	var idle := 0
	for step in maximum_steps:
		if Time.get_ticks_usec() - started >= budget_usec:
			break
		scheduler_cursor %= active.size()
		var runtime = active[scheduler_cursor]
		scheduler_cursor = (scheduler_cursor + 1) % active.size()
		if runtime._job_step():
			idle = 0
		else:
			idle += 1
			if idle >= active.size():
				break

func _prepare_frame(delta: float) -> void:
	tick_elapsed = minf(tick_elapsed + maxf(0,delta),1.0)
	_dispatch_world_events()
	if not tick_queued:
		var event := {"dt":tick_elapsed}
		if scope.has_method("creator_tracking"):
			event.tracking = scope.creator_tracking()
		if _enqueue("tick",event):
			tick_elapsed = 0
			tick_queued = true

func _enqueue(event: String, data: Dictionary) -> bool:
	var encoded := JSON.stringify(data)
	var bytes := encoded.to_utf8_buffer().size()
	if jobs.size() >= 128 or bytes > 262144 or job_bytes + bytes > 2 * 1024 * 1024:
		script_error.emit("Client creator dispatch backlog exceeded; newest event dropped")
		return false
	jobs.append({"event":event,"data":encoded,"bytes":bytes,"script":0,"commands":[],"command":0,"attachment":"","refreshed":false})
	job_bytes += bytes
	return true

func _refresh_context() -> void:
	if avatar_queries:
		if scripts.any(func(script): return not script.failed and script.get("queries", false)):
			_refresh_avatar_queries()
		else:
			avatar_queries = false
			scene.clear()

func _script_commands(script: Dictionary, event: String, encoded: String) -> Array:
	if script.failed:
		return []
	var response = JSON.parse_string(script.vm.dispatch(event,encoded))
	if not response is Dictionary:
		return []
	if response.has("error"):
		script.failed = true
		script_error.emit(str(response.error))
		return []
	return response.get("commands",[])

func _job_step() -> bool:
	if jobs.is_empty() or not is_instance_valid(scope):
		return false
	var job: Dictionary = jobs[0]
	if not job.refreshed:
		_refresh_context()
		job.refreshed = true
		return true
	if int(job.command) < job.commands.size():
		_apply(job.commands[int(job.command)],str(job.attachment))
		job.command = int(job.command) + 1
		return true
	if int(job.script) < scripts.size():
		var script: Dictionary = scripts[int(job.script)]
		job.script = int(job.script) + 1
		job.commands = _script_commands(script,str(job.event),str(job.data))
		job.command = 0
		job.attachment = script.attachment
		return true
	job_bytes -= int(job.bytes)
	if job.event == "tick":
		tick_queued = false
	jobs.pop_front()
	return true

func _refresh_avatar_queries() -> void:
	var snapshot: Dictionary = scope.creator_query_snapshot()
	if snapshot.has("error"):
		_scene_result(str(snapshot.error))
	else:
		_scene_result(scene.update_objects(JSON.stringify(snapshot.objects).to_utf8_buffer(), Time.get_ticks_usec()))

func queue_world_events(events: Array) -> void:
	if scripts.is_empty() and not world_queries:
		return
	for event in events:
		if not event is Dictionary or event.get("event") not in ["grab", "lease", "release", "damage", "destroy", "join", "leave", "collision", "attach", "detach"]:
			continue
		if pending_events.size() >= 128:
			event_overflow = true
			continue
		pending_events.append(event.duplicate(true))

func _dispatch_world_events() -> void:
	if not is_instance_valid(scope) or scripts.is_empty():
		return
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
		pending_events.clear()
		event_overflow = false
		world_queries = false
		if scene:
			scene.clear()
		return
	if not ClassDB.class_exists("CreatorVm") or not _ensure_scene():
		if manifest.get("scripts", []).any(func(s): return s.get("side") == "client"):
			script_error.emit("Client Luau extension is unavailable; use the Nix-packaged client")
		return
	avatar_queries = not world_queries and root.has_method("creator_query_snapshot") and manifest.get("scripts", []).any(func(script): return script.get("side") == "client" and "queries" in script.get("capabilities", []))
	if avatar_queries:
		_refresh_avatar_queries()
	var zip := ZIPReader.new()
	if zip.open(path) != OK:
		pending_events.clear()
		event_overflow = false
		script_error.emit("Could not read creator package")
		return
	for declaration in manifest.get("scripts", []):
		if declaration.side != "client":
			continue
		var source := zip.read_file(str(declaration.path)).get_string_from_utf8()
		var vm = ClassDB.instantiate("CreatorVm")
		var queries: bool = "queries" in declaration.capabilities
		if world_queries or queries:
			vm.bind_scene(scene)
		var error: String = vm.load_script(source, PackedStringArray(declaration.capabilities))
		if error.is_empty():
			scripts.append({"vm":vm,"attachment":str(declaration.attachment),"failed":false,"queries":queries})
		else:
			script_error.emit(error)
	zip.close()
	# Drain bounded top-level commands separately, before membership/physics
	# callbacks or the first tick consume their own dispatch command budget.
	dispatch("__initialize", {})
	if scripts.is_empty():
		pending_events.clear()
		event_overflow = false

func dispatch(event: String, data: Dictionary) -> void:
	if not is_instance_valid(scope):
		return
	# Runtime callbacks enter the fair bounded scheduler. Explicitly paused or
	# detached fixtures retain a synchronous host path for deterministic tooling.
	if event != "__initialize" and is_inside_tree() and is_processing():
		_enqueue(event,data)
		return
	_refresh_context()
	var encoded := JSON.stringify(data)
	for script in scripts:
		for command in _script_commands(script,event,encoded):
			_apply(command, script.attachment)

func _apply(command: Dictionary, attachment: String) -> void:
	# Exact node names, not paths: neither '..' nor an absolute path can escape scope.
	var target := str(command.get("target", ""))
	if command.action == "ambient":
		var host = scope.get_meta("ssc_visual_host",null)
		if attachment != "world" or not is_instance_valid(host) or not host.has_method("creator_ambient"):
			script_error.emit("Only world-attached client scripts may control global ambient tint")
		else:
			host.creator_ambient(command.color,float(command.brightness))
	elif command.action in ["material","light"]:
		var error := Visuals.apply(scope,attachment,command)
		if not error.is_empty():
			script_error.emit(error)
	elif command.action in ["play_animation","stop_animation","seek_animation"]:
		var error := preload("res://scripts/creator_animations.gd").apply(scope,attachment,command)
		if not error.is_empty():
			script_error.emit(error)
	elif command.action == "media" and scope.has_meta("ssc_network") and (attachment == "world" or attachment == target):
		var now := Time.get_ticks_msec()
		if now - media_window >= 1000:
			media_window = now
			media_requests = 0
			media_overflow = false
		if media_requests >= 8:
			if not media_overflow:
				media_overflow = true
				script_error.emit("Creator media request rate exceeded")
			return
		media_requests += 1
		var network: Node = scope.get_meta("ssc_network")
		network.command(22, command.request)
	elif command.action == "radio" and scope.has_meta("ssc_radio"):
		var radio: Node = scope.get_meta("ssc_radio")
		radio.transmit(target, str(command.mode), str(command.data))
	elif command.action == "zone":
		var acoustics = scope.get_meta("ssc_acoustics",null)
		if attachment != "world" or not is_instance_valid(acoustics):
			script_error.emit("Only world-attached client scripts may configure acoustic zones")
		else:
			var error: String = acoustics.creator_property(target,str(command.property),command.value)
			if not error.is_empty():
				script_error.emit(error)
	elif command.action in ["play_sound", "stop_sound", "seek_sound"] and scope.has_meta("ssc_audio"):
		var audio: Node = scope.get_meta("ssc_audio")
		match command.action:
			"play_sound": audio.play_sound(target)
			"stop_sound": audio.stop_sound(target)
			"seek_sound": audio.seek_sound(target, float(command.value))
	elif command.action == "visible":
		if target.begins_with("node:") and scope.has_method("creator_node"):
			var node: Node3D = scope.creator_node(target)
			if node and attachment == "avatar":
				node.visible = bool(command.value)
			return
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
	advance_all(delta,Engine.get_process_frames())

func close(clear_snapshot: bool = true) -> void:
	for script in scripts:
		script.vm.close()
	scripts.clear()
	media_window = 0
	media_requests = 0
	media_overflow = false
	avatar_queries = false
	jobs.clear()
	job_bytes = 0
	tick_elapsed = 0
	tick_queued = false
	scope = null
	if clear_snapshot:
		pending_events.clear()
		event_overflow = false
		world_queries = false
		scene_error = ""
		if scene:
			scene.clear()

func _exit_tree() -> void:
	var retained: Array[WeakRef] = []
	for reference in runtimes:
		if reference.get_ref() != self:
			retained.append(reference)
	runtimes = retained
	close()
