extends Node
## Only detached JSON and capability-filtered visual commands cross the VM boundary.
signal script_error(message: String)
var scripts: Array[Dictionary] = []
var scope: Node
var disabled := false

func load_package(path: String, manifest: Dictionary, root: Node) -> void:
	close()
	scope = root
	if not ClassDB.class_exists("CreatorVm"):
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
	elif scope.has_method("creator_action"):
		scope.creator_action(str(command.action), target, float(command.get("value", 1)))

func _process(delta: float) -> void:
	var event := {"dt":delta}
	if is_instance_valid(scope) and scope.has_method("creator_tracking"):
		event.tracking = scope.creator_tracking()
	dispatch("tick", event)

func close() -> void:
	for script in scripts:
		script.vm.close()
	scripts.clear()
	scope = null

func _exit_tree() -> void:
	close()
