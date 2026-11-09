extends RefCounted
## Incremental ambient recoloring. Never reimport or renormalize the scene for a
## day/night update. A superseding request does not restart an unfinished walk.
const Materials = preload("res://scripts/materials.gd")
const STEPS_PER_FRAME := 256
var roots: Array[WeakRef] = []
var next_roots: Array[WeakRef] = []
var stack: Array[Dictionary] = []
var root_index := 0
var revision := 0
var walking_revision := 0

func request(nodes: Array, color: Color, brightness: float) -> void:
	Materials.ambient_tint = Color(color.r * brightness,color.g * brightness,color.b * brightness,1)
	next_roots.clear()
	for node in nodes:
		if is_instance_valid(node):
			next_roots.append(weakref(node))
	revision += 1
	if roots.is_empty() and stack.is_empty():
		_begin()

func _begin() -> void:
	roots = next_roots.duplicate()
	root_index = 0
	walking_revision = revision

func advance() -> void:
	for step in STEPS_PER_FRAME:
		if stack.is_empty():
			if root_index >= roots.size():
				roots.clear()
				if walking_revision != revision:
					_begin()
				else:
					return
			if root_index >= roots.size():
				return
			stack.append({"node":roots[root_index],"child":-1})
			root_index += 1
		var entry: Dictionary = stack.back()
		var node = entry.node.get_ref()
		if not is_instance_valid(node) or node.is_queued_for_deletion():
			stack.pop_back()
			continue
		if int(entry.child) == -1:
			if node is MeshInstance3D:
				Materials.apply_tint(node)
			entry.child = 0
		elif int(entry.child) < node.get_child_count():
			var child: Node = node.get_child(int(entry.child))
			entry.child = int(entry.child) + 1
			stack.append({"node":weakref(child),"child":-1})
		else:
			stack.pop_back()

func clear() -> void:
	roots.clear()
	next_roots.clear()
	stack.clear()
	root_index = 0
	revision = 0
	walking_revision = 0
	Materials.ambient_tint = Color.WHITE
