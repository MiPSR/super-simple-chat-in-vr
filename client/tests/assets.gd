extends SceneTree
const Runtime = preload("res://scripts/vrm_runtime.gd")
const Loader = preload("res://scripts/package_loader.gd")
const Entity = preload("res://scripts/entity.gd")
var runtime: RefCounted
var loader: Node
var entity: Node3D
var started := 0
var received := false

func _initialize() -> void:
	runtime = Runtime.new()
	assert(runtime.available(), "Pinned VRM importer is missing from the Nix client project")
	var config: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(OS.get_environment("SSC_TEST_CONFIG")))
	loader = Loader.new()
	root.add_child.call_deferred(loader)
	loader.staging = str(config.package)
	loader.expected = str(config.hash)
	loader.manifest = config.manifest
	loader.failed.connect(func(message: String): push_error(message); quit(1))
	loader.loaded.connect(func(model: Node3D):
		root.add_child(model)
		assert(not model.find_children("*", "MeshInstance3D", true, false).is_empty(), "World mesh did not import")
		assert(loader.prefabs.has("tank") and loader.prefabs.has("shell"), "Packaged prefabs did not import")
		entity = Entity.new()
		root.add_child(entity)
		entity.configure({"id":"tank", "component":"vehicle", "prefab":"tank", "position":[0,1,0], "rotation":[0,0,0,1], "size":[1.5,0.8,2], "health":100}, loader.prefabs, 0, 1)
		assert(not entity.visual.has_meta("fallback"), "Tank used a placeholder instead of its packaged mesh")
		received = true
		started = Time.get_ticks_msec()
	)
	loader._start_import.call_deferred()
	started = Time.get_ticks_msec()

func _process(_delta: float) -> bool:
	if received and Time.get_ticks_msec() - started > 300:
		assert(entity.position.distance_to(Vector3(0,1,0)) < 0.001, "Entity extrapolated beyond the last known sample")
		print("PASS world GLB, packaged prefab meshes, pinned VRM importer, and hold-last-sample presentation")
		runtime.close()
		quit()
	elif Time.get_ticks_msec() - started > 5000:
		quit(2)
	return false
