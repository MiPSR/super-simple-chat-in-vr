extends SkeletonModifier3D
## Final local constraint after the importer's spring solver. No bone streaming.
var constraints: Array[Dictionary] = []
var previous: Array[int] = []

func _process_modification() -> void:
	var skeleton := get_skeleton()
	if not skeleton:
		return
	var active_bones: Array[int] = []
	for constraint in constraints:
		var index := skeleton.find_bone(str(constraint.bone))
		if index < 0:
			continue
		active_bones.append(index)
		var parent := skeleton.get_bone_parent(index)
		var parent_pose := skeleton.get_bone_global_pose(parent) if parent >= 0 else Transform3D.IDENTITY
		var local: Vector3 = (skeleton.global_transform * parent_pose).affine_inverse() * constraint.position
		skeleton.set_bone_pose_position(index, local)
	for index in previous:
		if not active_bones.has(index):
			skeleton.set_bone_pose_position(index, skeleton.get_bone_rest(index).origin)
	previous = active_bones
