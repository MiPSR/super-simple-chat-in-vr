extends SceneTree
const Runtime = preload("res://scripts/vrm_runtime.gd")
const Loader = preload("res://scripts/package_loader.gd")
const Avatar = preload("res://scripts/avatar.gd")
const BodyTracking = preload("res://scripts/body_tracking.gd")
var runtime: RefCounted
var avatar: Node3D
var loader: Node
var started := 0
var loaded := false
var tracking: Dictionary = {}

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
	root.add_child.call_deferred(loader)
	loader.staging = str(config.package)
	loader.expected = str(config.hash)
	loader.manifest = config.manifest
	loader.failed.connect(func(message: String): push_error(message); quit(1))
	loader.loaded.connect(func(model: Node3D):
		assert(model.get("vrm_meta") != null, "VRM metadata missing")
		assert(model.has_node("secondary"), "Spring-bone integration missing")
		assert(model.get_node("secondary").get("spring_bones").size() > 0)
		avatar = Avatar.new()
		root.add_child(avatar)
		avatar.set_model(model, loader.staging, loader.manifest)
		assert(avatar.model.skeleton.get_bone_count() >= 16)
		assert(avatar.model.solvers.size() == 4, "Native arm/leg IK chains missing")
		avatar.set_actions({"happy":1.0})
		var changed := false
		var diagnostics: Array = []
		for mesh in model.find_children("*", "MeshInstance3D", true, false):
			var index: int = mesh.find_blend_shape_by_name("Happy")
			diagnostics.append({"name":mesh.name,"shape":index,"count":mesh.mesh.get_blend_shape_count()})
			if index >= 0 and mesh.get_blend_shape_value(index) > 0.5:
				changed = true
		assert(changed, "Replicated expression did not reach a blend shape: " + str(diagnostics))
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
			print("PASS VRM 1.0, expressions, spring bones, native limb IK, body-tracker inputs/fallback, and local/sleeping poses")
			runtime.close()
			quit()
	elif Time.get_ticks_msec() - started > 5000:
		quit(2)
	return false
