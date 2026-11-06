extends Node3D
const CosmeticGrab = preload("res://scripts/cosmetic_grab.gd")
## Native Godot limb IK over the importer's normalized humanoid bone mapping.
var skeleton: Skeleton3D
var targets: Dictionary = {}
var solvers: Array[SkeletonIK3D] = []
var bone_map: BoneMap
var head_bone := -1
var rest_head := Vector3(0, 1.65, 0)
var sources: Dictionary = {}
var model_root: Node3D
var floor_heights: Dictionary = {}
var limb_solvers: Dictionary = {}
var grab_modifier: SkeletonModifier3D

func setup(model: Node3D) -> void:
	model_root = model
	add_child(model)
	var found := model.find_children("*", "Skeleton3D", true, false)
	if found.is_empty():
		return
	skeleton = found[0]
	var meta = model.get("vrm_meta")
	if meta:
		bone_map = meta.get("humanoid_bone_mapping")
	head_bone = bone("Head")
	if head_bone >= 0:
		rest_head = skeleton.get_bone_global_rest(head_bone).origin
	for side in ["Left", "Right"]:
		_limb(side + "UpperArm", side + "Hand")
		_limb(side + "UpperLeg", side + "Foot")
	grab_modifier = CosmeticGrab.new()
	skeleton.add_child(grab_modifier)

func bone(name: String) -> int:
	var mapped := bone_map.get_skeleton_bone_name(name) if bone_map else StringName(name)
	var index := skeleton.find_bone(mapped)
	return index if index >= 0 else skeleton.find_bone(name)

func _limb(root_name: String, tip_name: String) -> void:
	var root := bone(root_name)
	var tip := bone(tip_name)
	if root < 0 or tip < 0:
		return
	var target := Node3D.new()
	add_child(target)
	targets[tip_name] = target
	var solver := SkeletonIK3D.new()
	solver.root_bone = skeleton.get_bone_name(root)
	solver.tip_bone = skeleton.get_bone_name(tip)
	skeleton.add_child(solver)
	solver.target_node = solver.get_path_to(target)
	solver.override_tip_basis = true
	solver.start()
	solvers.append(solver)
	limb_solvers[tip_name] = solver

func update_pose(poses: Array[Transform3D], sleeping: bool, tracked: Dictionary = {}) -> void:
	if not skeleton or poses.size() < 3:
		return
	var head := poses[0]
	var yaw := head.basis.get_euler().y
	global_basis = Basis(Vector3.UP, yaw)
	if sleeping:
		global_basis *= Basis(Vector3.RIGHT, PI / 2)
	global_position = head.origin - global_basis * rest_head
	sources.clear()
	for name in ["Hips", "Chest"]:
		var index := bone(name)
		if tracked.has(name) and index >= 0:
			var parent := skeleton.get_bone_parent(index)
			var parent_pose := skeleton.get_bone_global_pose(parent) if parent >= 0 else Transform3D.IDENTITY
			var pose: Transform3D = (skeleton.global_transform * parent_pose).affine_inverse() * tracked[name].transform
			skeleton.set_bone_pose_position(index, pose.origin)
			skeleton.set_bone_pose_rotation(index, pose.basis.get_rotation_quaternion())
			sources[name] = str(tracked[name].source)
		else:
			sources[name] = "estimated"
			if index >= 0:
				skeleton.set_bone_pose_position(index, skeleton.get_bone_rest(index).origin)
				skeleton.set_bone_pose_rotation(index, skeleton.get_bone_rest(index).basis.get_rotation_quaternion())
	for pair in [["LeftHand", 1], ["RightHand", 2]]:
		if targets.has(pair[0]):
			targets[pair[0]].global_transform = poses[pair[1]]
			sources[pair[0]] = "tracked"
	for side in ["Left", "Right"]:
		var name: String = side + "Foot"
		if not targets.has(name):
			continue
		if tracked.has(name):
			targets[name].global_transform = tracked[name].transform
			sources[name] = str(tracked[name].source)
		else:
			var offset := Vector3(-0.14 if side == "Left" else 0.14, -1.5, 0)
			var foot := head.origin + global_basis * offset
			if not sleeping:
				foot.y = float(floor_heights.get(name, 0.0)) + 0.05
			targets[name].global_transform = Transform3D(global_basis, foot)
			sources[name] = "estimated"
		var knee: String = side + "LowerLeg"
		if limb_solvers.has(name):
			limb_solvers[name].use_magnet = tracked.has(knee)
			if tracked.has(knee):
				limb_solvers[name].magnet = skeleton.to_local(tracked[knee].transform.origin)
				sources[knee] = str(tracked[knee].source)
			else:
				sources[knee] = "estimated"
	if head_bone >= 0:
		var parent := skeleton.get_bone_parent(head_bone)
		var parent_pose := skeleton.get_bone_global_pose(parent) if parent >= 0 else Transform3D.IDENTITY
		var local := (skeleton.global_transform * parent_pose).basis.inverse() * head.basis
		skeleton.set_bone_pose_rotation(head_bone, local.get_rotation_quaternion())
		sources.Head = "tracked"

func _physics_process(_delta: float) -> void:
	for name in targets:
		if str(name).ends_with("Foot"):
			var foot: Vector3 = targets[name].global_position
			var query := PhysicsRayQueryParameters3D.create(foot + Vector3.UP * 1.5, foot - Vector3.UP * 2)
			var hit := get_world_3d().direct_space_state.intersect_ray(query)
			if not hit.is_empty():
				floor_heights[name] = float(hit.position.y)

func creator_action(action: String, name: String, weight: float) -> void:
	if not model_root:
		return
	var players := model_root.find_children("*", "AnimationPlayer", true, false)
	if players.is_empty():
		return
	var player: AnimationPlayer = players[0]
	if not player.has_animation(name):
		return
	if action == "animation":
		player.play(name)
	elif action == "expression":
		var animation := player.get_animation(name)
		var root := player.get_node(player.root_node)
		for track in animation.get_track_count():
			if animation.track_get_type(track) != Animation.TYPE_BLEND_SHAPE or animation.track_get_key_count(track) == 0:
				continue
			var path := animation.track_get_path(track)
			var mesh := root.get_node_or_null(NodePath(path.get_concatenated_names())) as MeshInstance3D
			if mesh and path.get_subname_count() > 0:
				var index := mesh.find_blend_shape_by_name(path.get_subname(0))
				if index >= 0:
					mesh.set_blend_shape_value(index, float(animation.track_get_key_value(track,0)) * weight)

func creator_tracking() -> Dictionary:
	return sources.duplicate()

func grabs(constraints: Array[Dictionary]) -> void:
	if grab_modifier:
		grab_modifier.constraints = constraints
