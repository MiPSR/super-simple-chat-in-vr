extends RefCounted
const Materials = preload("res://scripts/materials.gd")

static func apply(root: Node, styles: Dictionary) -> String:
	# Broad @root first, then deterministic exact names. Uploads cannot preseed
	# host metadata; this is called only after Materials.prepare cleared extras.
	var names := styles.keys()
	names.sort()
	for name in names:
		var target: Node = root if str(name) == "@root" else null
		if str(name) != "@root":
			var candidates: Array[Node] = root.find_children("*","Node",true,false)
			candidates.append(root)
			for candidate in candidates:
				if str(candidate.name) == str(name):
					if target:
						return "Ambiguous material style target: " + str(name)
					target = candidate
		if not target:
			return "Missing material style target: " + str(name)
		var meshes: Array[Node] = target.find_children("*","MeshInstance3D",true,false)
		if target is MeshInstance3D:
			meshes.append(target)
		if meshes.is_empty():
			return "Material style target has no meshes: " + str(name)
		for mesh: MeshInstance3D in meshes:
			var values: Dictionary = mesh.get_meta("ssc_creator_material",{}).duplicate(true)
			values.merge(styles[name],true)
			mesh.set_meta("ssc_creator_material",values)
			Materials.apply_overrides(mesh)
	return ""
