extends SkeletonModifier3D
## Avatar-local absolute bone rotations, after IK/springs and before grab constraints.
## No tracking inputs, shared outcomes or another avatar's skeleton are changed.
var rotations: Dictionary = {}

func rotate_bone(index: int, rotation: Quaternion) -> String:
	if not rotations.has(index) and rotations.size() >= 64:
		return "At most 64 creator bone overrides per avatar"
	rotations[index] = rotation
	return ""

func reset_bone(index: int) -> void:
	rotations.erase(index)
	var skeleton := get_skeleton()
	if skeleton and index >= 0 and index < skeleton.get_bone_count():
		skeleton.set_bone_pose_rotation(index, skeleton.get_bone_rest(index).basis.get_rotation_quaternion())

func _process_modification() -> void:
	var skeleton := get_skeleton()
	if not skeleton:
		return
	for index: int in rotations:
		if index >= 0 and index < skeleton.get_bone_count():
			skeleton.set_bone_pose_rotation(index, rotations[index])
