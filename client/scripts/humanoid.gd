extends Node3D
const CosmeticGrab = preload("res://scripts/cosmetic_grab.gd")
const CreatorBones = preload("res://scripts/creator_bones.gd")
const BodyPose = preload("res://scripts/body_pose.gd")
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
var bone_modifier: SkeletonModifier3D
var query_nodes: Array[Node3D] = []
var query_bones: Array[Dictionary] = []
var last_head := Vector3(0, 1.65, 0)
var posture := "standing"
var limb_bones: Dictionary = {}
var animation_players: Array[Node] = []
var animation_bones: Dictionary = {}
var humanoid_bones: Dictionary = {}

func setup(model: Node3D, queries: bool = false) -> void:
	model_root = model
	if queries:
		# Capture supplied nodes before adding host IK targets/modifiers. Flat
		# indices remain stable when a supplied node is removed; never reuse IDs.
		query_nodes.append(model)
		for node in model.find_children("*", "Node3D", true, false):
			query_nodes.append(node as Node3D)
	# Upstream godot-vrm #157 workaround (see vrm_runtime.gd): rebind skinned
	# meshes once the model enters the tree and its skeleton binding settles.
	model.ready.connect(func(): preload("res://scripts/vrm_runtime.gd").invalidate_skeletons(model))
	add_child(model)
	var found := model.find_children("*", "Skeleton3D", true, false)
	if found.is_empty():
		return
	skeleton = found[0]
	animation_players = model.find_children("*", "AnimationPlayer", true, false)
	var meta = model.get("vrm_meta")
	if meta:
		bone_map = meta.get("humanoid_bone_mapping")
	if queries:
		var profile: SkeletonProfile = bone_map.profile if bone_map and bone_map.profile else SkeletonProfileHumanoid.new()
		for index in profile.bone_size:
			var canonical := str(profile.get_bone_name(index))
			var actual := bone(canonical)
			if actual >= 0:
				query_bones.append({"id":"humanoid:" + canonical,"index":actual})
	head_bone = bone("Head")
	if head_bone >= 0:
		rest_head = skeleton.get_bone_global_rest(head_bone).origin
	for name in ["Hips", "Spine", "Chest", "UpperChest", "Neck", "Head", "LeftShoulder", "LeftUpperArm", "LeftLowerArm", "LeftHand", "LeftUpperLeg", "LeftLowerLeg", "LeftFoot", "RightShoulder", "RightUpperArm", "RightLowerArm", "RightHand", "RightUpperLeg", "RightLowerLeg", "RightFoot"]:
		var index := bone(name)
		if index >= 0:
			humanoid_bones[name] = index
	for side in ["Left", "Right"]:
		_limb(side + "UpperArm", side + "Hand")
		_limb(side + "UpperLeg", side + "Foot")
	bone_modifier = CreatorBones.new()
	# The VRM importer places its spring modifier in INTERNAL_MODE_BACK.
	# Append to that same group so our final constraints really run after it.
	skeleton.add_child(bone_modifier, false, Node.INTERNAL_MODE_BACK)
	grab_modifier = CosmeticGrab.new()
	skeleton.add_child(grab_modifier, false, Node.INTERNAL_MODE_BACK)

func bone(name: String) -> int:
	# BoneMap logs an engine error for non-profile bones; ordinary authored
	# accessories/hair must use the exact-skeleton fallback without that call.
	var mapped := bone_map.get_skeleton_bone_name(name) if bone_map and bone_map.profile and bone_map.profile.find_bone(name) >= 0 else StringName(name)
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
	var chain: Array[int] = []
	var index := tip
	while index >= 0:
		chain.append(index)
		if index == root:
			break
		index = skeleton.get_bone_parent(index)
	limb_bones[tip_name] = chain

func _animated_bones() -> Dictionary:
	var active: Dictionary = {}
	for node in animation_players:
		var player := node as AnimationPlayer
		if not is_instance_valid(player) or not player.is_playing() or not player.has_animation(player.current_animation):
			continue
		var clip: Animation = player.get_animation(player.current_animation)
		var key := clip.get_instance_id()
		if not animation_bones.has(key):
			var indices: Array[int] = []
			var root: Node = player.get_node_or_null(player.root_node)
			for track in clip.get_track_count():
				if not clip.track_is_enabled(track) or clip.track_get_type(track) not in [Animation.TYPE_POSITION_3D, Animation.TYPE_ROTATION_3D, Animation.TYPE_SCALE_3D]:
					continue
				var path := clip.track_get_path(track)
				if root and path.get_subname_count() == 1 and root.get_node_or_null(NodePath(path.get_concatenated_names())) == skeleton:
					var index := skeleton.find_bone(path.get_subname(0))
					if index >= 0:
						indices.append(index)
			animation_bones[key] = indices
		for index in animation_bones[key]:
			active[index] = true
	if bone_modifier:
		for index in bone_modifier.rotations:
			active[index] = true
	return active

func update_pose(poses: Array[Transform3D], sleeping: bool, tracked: Dictionary = {}, sleep_pose: String = "flat") -> void:
	if not skeleton or poses.size() < 3:
		return
	var head := poses[0]
	last_head = head.origin
	var body := BodyPose.estimate(head, rest_head, floor_heights, sleeping, sleep_pose)
	global_basis = body.basis
	global_position = body.root
	posture = str(body.posture)
	var animated := _animated_bones()
	sources.clear()
	for name in humanoid_bones:
		sources[name] = "animation" if animated.has(humanoid_bones[name]) else "estimated"
	for name in ["Hips", "Chest"]:
		var index := bone(name)
		if tracked.has(name) and index >= 0:
			var parent := skeleton.get_bone_parent(index)
			var parent_pose := skeleton.get_bone_global_pose(parent) if parent >= 0 else Transform3D.IDENTITY
			var pose: Transform3D = (skeleton.global_transform * parent_pose).affine_inverse() * tracked[name].transform
			skeleton.set_bone_pose_position(index, pose.origin)
			skeleton.set_bone_pose_rotation(index, pose.basis.get_rotation_quaternion())
			sources[name] = str(tracked[name].source)
		elif animated.has(index):
			sources[name] = "animation"
		else:
			sources[name] = "estimated"
			if index >= 0:
				skeleton.set_bone_pose_position(index, skeleton.get_bone_rest(index).origin)
				skeleton.set_bone_pose_rotation(index, skeleton.get_bone_rest(index).basis.get_rotation_quaternion())
	for pair in [["LeftHand", 1], ["RightHand", 2]]:
		if targets.has(pair[0]):
			var animation_driven: bool = limb_bones[pair[0]].any(func(index): return animated.has(index))
			limb_solvers[pair[0]].interpolation = 0.0 if animation_driven else 1.0
			targets[pair[0]].global_transform = poses[pair[1]]
			sources[pair[0]] = "animation" if animation_driven else "tracked"
			if animation_driven:
				for name in humanoid_bones:
					if humanoid_bones[name] in limb_bones[pair[0]]:
						sources[name] = "animation"
	for side in ["Left", "Right"]:
		var name: String = side + "Foot"
		if not targets.has(name):
			continue
		var animation_driven: bool = limb_bones[name].any(func(index): return animated.has(index))
		limb_solvers[name].interpolation = 0.0 if animation_driven else 1.0
		if animation_driven:
			sources[name] = "animation"
			sources[side + "LowerLeg"] = "animation"
			for canonical in humanoid_bones:
				if humanoid_bones[canonical] in limb_bones[name]:
					sources[canonical] = "animation"
			continue
		if tracked.has(name):
			targets[name].global_transform = tracked[name].transform
			sources[name] = str(tracked[name].source)
		else:
			targets[name].global_transform = body.feet[name]
			sources[name] = "estimated"
		var knee: String = side + "LowerLeg"
		if limb_solvers.has(name):
			limb_solvers[name].use_magnet = true
			if tracked.has(knee):
				limb_solvers[name].magnet = skeleton.to_local(tracked[knee].transform.origin)
				sources[knee] = str(tracked[knee].source)
			else:
				limb_solvers[name].magnet = skeleton.to_local(body.knees[knee])
				sources[knee] = "estimated"
	if head_bone >= 0 and animated.has(head_bone):
		sources.Head = "animation"
	elif head_bone >= 0:
		var parent := skeleton.get_bone_parent(head_bone)
		var parent_pose := skeleton.get_bone_global_pose(parent) if parent >= 0 else Transform3D.IDENTITY
		var local := (skeleton.global_transform * parent_pose).basis.inverse() * head.basis
		skeleton.set_bone_pose_rotation(head_bone, local.get_rotation_quaternion())
		sources.Head = "tracked"

func _physics_process(_delta: float) -> void:
	var feet: Dictionary = {}
	for name in targets:
		if str(name).ends_with("Foot"):
			feet[name] = targets[name].global_transform
	floor_heights = BodyPose.ground(get_world_3d().direct_space_state, feet, last_head, rest_head.y)

func creator_visual_root() -> Node3D:
	return model_root

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

func creator_bone(command: Dictionary) -> String:
	if not skeleton or not bone_modifier:
		return "Avatar has no editable skeleton"
	var target := str(command.target)
	var index := _query_index(target, "bone:", skeleton.get_bone_count()) if target.begins_with("bone:") else bone(target.trim_prefix("humanoid:"))
	if index < 0:
		return "Unknown avatar bone: " + str(command.target)
	if command.action == "reset_bone":
		bone_modifier.reset_bone(index)
		return ""
	var values: Array = command.rotation
	return bone_modifier.rotate_bone(index, Quaternion(float(values[0]), float(values[1]), float(values[2]), float(values[3])))

static func _query_index(id: String, prefix: String, count: int) -> int:
	if not id.begins_with(prefix):
		return -1
	var text := id.trim_prefix(prefix)
	if text.length() > 10 or not text.is_valid_int():
		return -1
	var index := int(text)
	return index if index >= 0 and index < count and str(index) == text else -1

func creator_node(id: String) -> Node3D:
	var index := _query_index(id, "node:", query_nodes.size())
	if index >= 0 and is_instance_valid(query_nodes[index]):
		return query_nodes[index]
	return null

func _bone_snapshot(index: int, tag: String) -> Dictionary:
	var position := skeleton.get_bone_pose_position(index)
	var rotation := skeleton.get_bone_pose_rotation(index)
	return {"component":"bone","name":str(skeleton.get_bone_name(index)).left(64),"tags":[tag],"position":[position.x,position.y,position.z],"rotation":[rotation.x,rotation.y,rotation.z,rotation.w]}

func creator_query_snapshot() -> Dictionary:
	var bone_count := skeleton.get_bone_count() if skeleton else 0
	if query_nodes.size() + bone_count + query_bones.size() > 4096:
		return {"error":"Avatar query context exceeds 4096 objects"}
	var objects: Dictionary = {}
	for index in query_nodes.size():
		var node = query_nodes[index]
		if not is_instance_valid(node):
			continue
		var position: Vector3 = node.position
		var basis: Basis = node.transform.basis
		if not position.is_finite() or not basis.is_finite():
			return {"error":"Non-finite avatar query transform"}
		var rotation: Quaternion = node.quaternion if absf(basis.determinant()) > 0.000001 else Quaternion.IDENTITY
		objects["node:%s" % index] = {"component":"node","name":str(node.name).left(64),"tags":["avatar.node"],"visible":node.visible,"position":[position.x,position.y,position.z],"rotation":[rotation.x,rotation.y,rotation.z,rotation.w]}
	for index in bone_count:
		objects["bone:%s" % index] = _bone_snapshot(index, "avatar.bone")
	for entry in query_bones:
		objects[str(entry.id)] = _bone_snapshot(int(entry.index), "avatar.humanoid")
	return {"objects":objects}

func creator_spring_gravity(multiplier: float) -> String:
	if not is_finite(multiplier) or multiplier < -2 or multiplier > 2:
		return "Invalid spring gravity multiplier"
	var secondary := model_root.get_node_or_null("secondary") if model_root else null
	if not secondary or secondary.get("springbone_gravity_multiplier") == null:
		return "Avatar has no supported spring system"
	secondary.set("springbone_gravity_multiplier", multiplier)
	return ""

func creator_tracking() -> Dictionary:
	return sources.duplicate()

func creator_posture() -> String:
	return posture

func grabs(constraints: Array[Dictionary]) -> void:
	if grab_modifier:
		grab_modifier.constraints = constraints
