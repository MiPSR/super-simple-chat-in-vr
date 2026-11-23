extends RefCounted
const BodyPose = preload("res://scripts/body_pose.gd")
const Tracking = preload("res://scripts/body_tracking.gd")
const Humanoid = preload("res://scripts/humanoid.gd")

static func tracker() -> XRBodyTracker:
	var body := XRBodyTracker.new()
	for joint in XRBodyTracker.JOINT_MAX:
		body.set_joint_flags(joint, 0)
	body.has_tracking_data = true
	return body

static func add_bone(skeleton: Skeleton3D, name: String, parent: int, position: Vector3) -> int:
	var index := skeleton.get_bone_count()
	skeleton.add_bone(name)
	skeleton.set_bone_parent(index, parent)
	skeleton.set_bone_rest(index, Transform3D(Basis.IDENTITY, position))
	skeleton.reset_bone_pose(index)
	return index

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var stature := Vector3(0, 1.65, 0)
	for entry in [[1.65, "standing"], [1.25, "crouching"], [0.8, "sitting"]]:
		var head := Transform3D(Basis.IDENTITY, Vector3(0, float(entry[0]), 0))
		var body := BodyPose.estimate(head, stature, {}, false, "flat")
		assert(body.posture == entry[1])
		assert(is_equal_approx(body.feet.LeftFoot.origin.y, 0.04))
		assert(body.knees.LeftLowerLeg.is_finite())
	var elevated := BodyPose.estimate(Transform3D(Basis.IDENTITY, Vector3(0, 4.65, 0)), stature, {"LeftFoot":3.0,"RightFoot":3.0}, false, "flat")
	assert(elevated.posture == "standing" and is_equal_approx(elevated.feet.LeftFoot.origin.y, 3.04))
	var sleeping := BodyPose.estimate(Transform3D(Basis.IDENTITY, Vector3(0, 0.3, 0)), stature, {}, true, "flat")
	var side := BodyPose.estimate(Transform3D(Basis.IDENTITY, Vector3(0, 0.3, 0)), stature, {}, true, "side")
	assert(sleeping.posture == "sleeping_flat" and side.posture == "sleeping_side" and not sleeping.basis.is_equal_approx(side.basis))
	var vertical := BodyPose.estimate(Transform3D(Basis(Vector3.RIGHT, PI / 2), stature), stature, {}, false, "flat")
	assert(vertical.basis.is_finite() and vertical.root.is_finite())
	# Merge partial trackers instead of throwing away every tracker but the
	# first; real tracked joints take precedence over inferred joints.
	var estimated := tracker()
	var actual := tracker()
	var valid := XRBodyTracker.JOINT_FLAG_POSITION_VALID | XRBodyTracker.JOINT_FLAG_ORIENTATION_VALID
	var tracked := valid | XRBodyTracker.JOINT_FLAG_POSITION_TRACKED | XRBodyTracker.JOINT_FLAG_ORIENTATION_TRACKED
	for joint in [XRBodyTracker.JOINT_LEFT_FOOT, XRBodyTracker.JOINT_RIGHT_FOOT]:
		estimated.set_joint_flags(joint, valid)
		estimated.set_joint_transform(joint, Transform3D.IDENTITY)
	actual.set_joint_flags(XRBodyTracker.JOINT_LEFT_FOOT, tracked)
	actual.set_joint_transform(XRBodyTracker.JOINT_LEFT_FOOT, Transform3D.IDENTITY)
	var merged := Tracking.new().from_trackers([estimated, actual], Transform3D.IDENTITY)
	assert(merged.size() == 2 and merged.LeftFoot.source == "tracked" and merged.RightFoot.source == "estimated")
	assert(Tracking.decode({"LeftFoot":{"pose":[0,0,0,0,0,0,1],"source":"tracked"}}).size() == 1)
	for bad in ["bad", [0,0,0,0,0,0,0], [NAN,0,0,0,0,0,1], ["x",0,0,0,0,0,1], [100000,0,0,0,0,0,1]]:
		assert(Tracking.decode({"LeftFoot":{"pose":bad}}).is_empty())
	assert(Tracking.decode({"LeftFoot":{"pose":[0,0,0,0,0,0,1],"source":"arbitrary"}}).is_empty())
	# Test real native limb solvers and supplied bone animations.
	var model := Node3D.new()
	var skeleton := Skeleton3D.new()
	skeleton.name = "Rig"
	model.add_child(skeleton)
	var hips := add_bone(skeleton, "Hips", -1, Vector3(0, 1, 0))
	var chest := add_bone(skeleton, "Chest", hips, Vector3(0, 0.4, 0))
	add_bone(skeleton, "Head", chest, Vector3(0, 0.25, 0))
	for name in ["Left", "Right"]:
		var sign_value := -1 if name == "Left" else 1
		var arm := add_bone(skeleton, name + "UpperArm", chest, Vector3(0.15 * sign_value, 0.05, 0))
		var elbow := add_bone(skeleton, name + "LowerArm", arm, Vector3(0.25 * sign_value, 0, 0))
		add_bone(skeleton, name + "Hand", elbow, Vector3(0.25 * sign_value, 0, 0))
		var thigh := add_bone(skeleton, name + "UpperLeg", hips, Vector3(0.12 * sign_value, 0, 0))
		var knee := add_bone(skeleton, name + "LowerLeg", thigh, Vector3(0, -0.45, 0))
		add_bone(skeleton, name + "Foot", knee, Vector3(0, -0.5, 0))
	var player := AnimationPlayer.new()
	model.add_child(player)
	var clip := Animation.new()
	clip.length = 1
	clip.loop_mode = Animation.LOOP_LINEAR
	var rotation := Quaternion(Vector3.UP, 0.4)
	for name in ["Hips", "LeftUpperLeg"]:
		var track := clip.add_track(Animation.TYPE_ROTATION_3D)
		clip.track_set_path(track, NodePath("Rig:" + name))
		clip.rotation_track_insert_key(track, 0, rotation)
	var library := AnimationLibrary.new()
	library.add_animation("pose", clip)
	player.add_animation_library("", library)
	var humanoid := Humanoid.new()
	tree.root.add_child(humanoid)
	humanoid.setup(model)
	var poses: Array[Transform3D] = [Transform3D(Basis.IDENTITY, stature), Transform3D(Basis.IDENTITY, Vector3(-0.4,1.2,-0.3)), Transform3D(Basis.IDENTITY, Vector3(0.4,1.2,-0.3))]
	humanoid.update_pose(poses, false)
	assert(humanoid.limb_solvers.LeftFoot.use_magnet and humanoid.sources.LeftFoot == "estimated")
	player.play("pose")
	player.advance(0)
	humanoid.update_pose(poses, false)
	assert(humanoid.sources.Hips == "animation" and humanoid.sources.LeftFoot == "animation")
	assert(is_zero_approx(humanoid.limb_solvers.LeftFoot.interpolation))
	assert(skeleton.get_bone_pose_rotation(hips).is_equal_approx(rotation), "Pose fallback overwrote the authored hips animation")
	player.stop()
	humanoid.update_pose(poses, false)
	assert(humanoid.sources.Hips == "estimated" and humanoid.limb_solvers.LeftFoot.interpolation == 1.0)
	assert(humanoid.creator_posture() == "standing")
	humanoid.free()
	var fallback = preload("res://scripts/avatar.gd").new()
	tree.root.add_child(fallback)
	fallback.local_pose(0.016, poses, false, {
		"Hips":{"pose":[0.3,0.9,0,0,0,0,1],"source":"tracked"},
		"LeftFoot":{"pose":[-0.3,0.1,0.2,0,0,0,1],"source":"tracked"},
	})
	assert(is_equal_approx(fallback.torso.position.x, 0.3))
	assert(fallback.legs[0].position.is_equal_approx(Vector3(-0.3,0.1,0.2)))
	fallback.free()
	# Sample actual collision geometry at an elevated floor, then remove it:
	# a missed ray must clear the old platform's floor, not retain it forever.
	var scene := Node3D.new()
	tree.root.add_child(scene)
	var floor := StaticBody3D.new()
	scene.add_child(floor)
	floor.position.y = 2.9
	var collider := CollisionShape3D.new()
	var shape := BoxShape3D.new()
	shape.size = Vector3(10, 0.2, 10)
	collider.shape = shape
	floor.add_child(collider)
	await tree.physics_frame
	await tree.physics_frame
	var floors := BodyPose.ground(scene.get_world_3d().direct_space_state, elevated.feet, Vector3(0,4.65,0), 1.65)
	assert(is_equal_approx(floors.LeftFoot, 3.0))
	floor.free()
	await tree.physics_frame
	assert(BodyPose.ground(scene.get_world_3d().direct_space_state, elevated.feet, Vector3(0,4.65,0), 1.65).is_empty())
	scene.free()
	print("PASS standing/crouching/sitting/sleeping fallback, elevated/retired floors, tracker merging and animation-driven joint sources")
