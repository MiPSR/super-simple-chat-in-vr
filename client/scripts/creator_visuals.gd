extends RefCounted
## Exact scoped IDs and fixed setters only. No paths, resource loads or Object.set.

static func target(scope: Node, id: String, attachment: String) -> Node:
	if attachment not in ["world","avatar",id] or id.is_empty() or "/" in id or id == "..":
		return null
	if id.begins_with("node:") and scope.has_method("creator_node"):
		return scope.creator_node(id) if attachment == "avatar" else null
	if id == "@world" and attachment == "world" and scope.has_meta("ssc_network"):
		return scope
	if id == "@avatar" and attachment == "avatar" and scope.has_method("creator_visual_root"):
		return scope.creator_visual_root()
	# Never resolve names against host IK/constraint helpers on an avatar.
	if scope.has_method("creator_visual_root"):
		scope = scope.creator_visual_root()
		if not is_instance_valid(scope):
			return null
	if scope.has_meta("ssc_entities"):
		var entities: Dictionary = scope.get_meta("ssc_entities")
		if entities.has(id) and is_instance_valid(entities[id]):
			return entities[id]
	if not scope.has_meta("ssc_visual_names"):
		var names: Dictionary = {}
		var nodes: Array[Node] = [scope]
		nodes.append_array(scope.find_children("*","Node",true,false))
		for node in nodes:
			var name := str(node.name)
			# Keep duplicates ambiguous even if one is subsequently removed.
			names[name] = null if names.has(name) else weakref(node)
		scope.set_meta("ssc_visual_names",names)
	var names: Dictionary = scope.get_meta("ssc_visual_names")
	var reference = names.get(id)
	var resolved: Node = reference.get_ref() if reference is WeakRef else null
	return resolved if is_instance_valid(resolved) and not resolved.is_queued_for_deletion() else null

static func descendants(node: Node, kind: String) -> Array[Node]:
	var key := "ssc_visual_" + kind
	if not node.has_meta(key):
		var references: Array[WeakRef] = []
		if node.is_class(kind):
			references.append(weakref(node))
		for child in node.find_children("*",kind,true,false):
			references.append(weakref(child))
		node.set_meta(key,references)
	var result: Array[Node] = []
	for reference: WeakRef in node.get_meta(key):
		var child = reference.get_ref()
		if is_instance_valid(child) and not child.is_queued_for_deletion():
			result.append(child)
	return result

static func apply(scope: Node, attachment: String, command: Dictionary) -> String:
	var node := target(scope,str(command.target),attachment)
	if not node:
		return "Unknown, ambiguous or out-of-scope visual target"
	if command.action == "material":
		if str(command.property) not in ["albedo","emission","emission_energy","roughness","metallic","specular","rim","rim_tint","outline_width","outline_color","matcap","matcap_strength"]:
			return "Unsupported material property"
		var meshes := descendants(node,"MeshInstance3D")
		if meshes.is_empty():
			return "Visual target has no normalized mesh materials"
		var style: Dictionary = node.get_meta("ssc_creator_material", {}).duplicate(true)
		style[str(command.property)] = command.value
		node.set_meta("ssc_creator_material",style)
		for mesh: MeshInstance3D in meshes:
			var overrides: Dictionary = mesh.get_meta("ssc_creator_material", {}).duplicate(true)
			overrides[str(command.property)] = command.value
			mesh.set_meta("ssc_creator_material",overrides)
			preload("res://scripts/materials.gd").apply_overrides(mesh)
		return ""
	if str(command.property) not in ["color","energy","enabled","shadow","range"]:
		return "Unsupported light property"
	var lights := descendants(node,"Light3D")
	if lights.is_empty():
		return "Visual target has no supported lights"
	if command.property == "range" and lights.any(func(light): return not light is OmniLight3D and not light is SpotLight3D):
		return "Directional lights have no range"
	var style: Dictionary = node.get_meta("ssc_creator_light",{}).duplicate(true)
	style[str(command.property)] = command.value
	node.set_meta("ssc_creator_light",style)
	for light in lights:
		var overrides: Dictionary = light.get_meta("ssc_creator_light",{}).duplicate(true)
		overrides[str(command.property)] = command.value
		light.set_meta("ssc_creator_light",overrides)
		preload("res://scripts/materials.gd").apply_light_overrides(light)
	return ""
