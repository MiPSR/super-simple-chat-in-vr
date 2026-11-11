extends SceneTree
const Runtime = preload("res://scripts/vrm_runtime.gd")
const Loader = preload("res://scripts/package_loader.gd")
const Avatar = preload("res://scripts/avatar.gd")
const BodyTracking = preload("res://scripts/body_tracking.gd")
const Cache = preload("res://scripts/asset_cache.gd")
var runtime: RefCounted
var avatar: Node3D
var loader: Node
var started := 0
var loaded := false
var tracking: Dictionary = {}
var bone_checks := 0

func _initialize() -> void:
	started = Time.get_ticks_msec()
	runtime = Runtime.new()
	var tracker := XRBodyTracker.new()
	# XRBodyTracker does not initialize every joint flag in its constructor.
	# Synthetic fixtures, like a tracker driver, must explicitly publish validity.
	for joint in XRBodyTracker.JOINT_MAX:
		tracker.set_joint_flags(joint, 0)
	tracker.has_tracking_data = true
	var flags := XRBodyTracker.JOINT_FLAG_POSITION_VALID | XRBodyTracker.JOINT_FLAG_ORIENTATION_VALID | XRBodyTracker.JOINT_FLAG_POSITION_TRACKED | XRBodyTracker.JOINT_FLAG_ORIENTATION_TRACKED
	tracker.set_joint_flags(XRBodyTracker.JOINT_LEFT_FOOT, flags)
	tracker.set_joint_transform(XRBodyTracker.JOINT_LEFT_FOOT, Transform3D(Basis.IDENTITY, Vector3(-0.2,0.1,0)))
	tracking = BodyTracking.new().from_tracker(tracker, Transform3D.IDENTITY)
	assert(tracking.has("LeftFoot") and tracking.LeftFoot.source == "tracked")
	assert(tracking.size() == 1, "Unconfigured synthetic joints appeared valid: " + str(tracking))
	var config: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(OS.get_environment("SSC_TEST_CONFIG")))
	loader = Loader.new()
	loader.cache_directory = "user://avatar-import-%s-%s" % [OS.get_process_id(),Time.get_ticks_usec()]
	assert(DirAccess.make_dir_recursive_absolute(loader.cache_directory) == OK)
	root.add_child.call_deferred(loader)
	loader.staging = loader.cache_directory.path_join(str(config.hash) + ".zip")
	assert(DirAccess.copy_absolute(str(config.package),loader.staging) == OK)
	loader.expected = str(config.hash)
	loader.manifest = config.manifest
	loader.failed.connect(func(message: String): push_error(message); quit(1))
	loader.loaded.connect(func(model: Node3D):
		assert(model.get("vrm_meta") != null, "VRM metadata missing")
		assert(model.has_node("secondary"), "Spring-bone integration missing")
		assert(model.get_node("secondary").get("spring_bones").size() > 0)
		avatar = Avatar.new()
		root.add_child(avatar)
		avatar.runtime_status.connect(func(message: String): push_error(message); quit(1))
		avatar.set_model(model, loader.staging, loader.manifest)
		assert(avatar.creator.scripts.size() == 1 and not avatar.creator.scripts[0].failed)
		assert(avatar.model.skeleton.get_bone_count() >= 16)
		assert(avatar.model.solvers.size() == 4, "Native arm/leg IK chains missing")
		var spring_index := -1
		for node in avatar.model.skeleton.get_children(true):
			if node.name == "VRM_internal_skeleton_modifier":
				spring_index = node.get_index(true)
		assert(spring_index >= 0, "VRM spring modifier missing")
		assert(avatar.model.bone_modifier.get_index(true) > spring_index)
		assert(avatar.model.grab_modifier.get_index(true) > avatar.model.bone_modifier.get_index(true))
		avatar.set_actions({"happy":1.0})
		var changed := false
		var diagnostics: Array = []
		for mesh in model.find_children("*", "MeshInstance3D", true, false):
			var index: int = mesh.find_blend_shape_by_name("Happy")
			diagnostics.append({"name":mesh.name,"shape":index,"count":mesh.mesh.get_blend_shape_count()})
			if index >= 0 and mesh.get_blend_shape_value(index) > 0.5:
				changed = true
		assert(changed, "Replicated expression did not reach a blend shape: " + str(diagnostics))
		var head: int = avatar.model.bone("Head")
		assert(head >= 0)
		assert(avatar.model.creator_bone({"action":"bone_rotation","target":"Head","rotation":[0,1,0,0]}).is_empty())
		avatar.model.bone_modifier._process_modification()
		assert(avatar.model.skeleton.get_bone_pose_rotation(head).is_equal_approx(Quaternion(0,1,0,0)))
		assert(avatar.model.creator_bone({"action":"reset_bone","target":"Head"}).is_empty())
		assert(avatar.model.bone_modifier.rotations.is_empty())
		avatar.creator._apply({"action":"spring_gravity","value":0.25}, "avatar")
		assert(is_equal_approx(float(model.get_node("secondary").get("springbone_gravity_multiplier")), 0.25))
		avatar.creator._apply({"action":"spring_gravity","value":1.0}, "world")
		assert(is_equal_approx(float(model.get_node("secondary").get("springbone_gravity_multiplier")), 0.25))
		avatar.creator._apply({"action":"spring_gravity","value":1.0}, "avatar")
		var hair: int = avatar.model.bone("HairRoot")
		assert(hair >= 0)
		assert(avatar.model.creator_bone({"action":"reset_bone","target":"NotAHumanoidBone"}).begins_with("Unknown"))
		assert(avatar.model.creator_bone({"action":"bone_rotation","target":"HairRoot","rotation":[0,1,0,0]}).is_empty())
		avatar.model.grab_modifier.modification_processed.connect(func():
			assert(avatar.model.skeleton.get_bone_pose_rotation(hair).is_equal_approx(Quaternion(0,1,0,0)), "Spring solver overwrote the creator bone rotation")
			bone_checks += 1
		)
		loaded = true
		started = Time.get_ticks_msec()
	)
	loader._start_import.call_deferred()

func _process(delta: float) -> bool:
	if loaded:
		var poses: Array[Transform3D] = [Transform3D(Basis.IDENTITY,Vector3(0,1.65,0)),Transform3D(Basis.IDENTITY,Vector3(-0.4,1.2,-0.3)),Transform3D(Basis.IDENTITY,Vector3(0.4,1.2,-0.3))]
		avatar.local_pose(delta,poses,Time.get_ticks_msec()-started>100,tracking)
		assert(avatar.model.global_position.is_finite())
		if avatar.model.sources.get("LeftFoot") != "tracked" or avatar.model.sources.get("RightFoot") != "estimated":
			push_error("Tracker/fallback sources differ: " + str(avatar.model.sources) + " inputs: " + str(tracking))
			quit(1)
			return false
		if Time.get_ticks_msec() - started > 300:
			assert(bone_checks > 0, "Imported skeleton modifiers never processed")
			print("PASS VRM expressions/springs, creator bones after springs, native IK, tracker fallback and sleeping poses")
			assert(Cache.remove_completed(loader.staging) == 0)
			assert(DirAccess.remove_absolute(loader.cache_directory.path_join(".ssc-cache.lock")) == OK)
			assert(DirAccess.remove_absolute(loader.cache_directory) == OK)
			runtime.close()
			quit()
	elif Time.get_ticks_msec() - started > 5000:
		quit(2)
	return false
