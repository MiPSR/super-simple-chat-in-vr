extends Node3D
signal runtime_status(message: String)
const Humanoid = preload("res://scripts/humanoid.gd")
const CreatorRuntime = preload("res://scripts/creator_runtime.gd")
const PackagedAudio = preload("res://scripts/packaged_audio.gd")
const BodyTracking = preload("res://scripts/body_tracking.gd")
## Bounded interpolation of received samples only. No extrapolation or rollback.
var samples: Array[Dictionary] = []
var head: MeshInstance3D
var hands: Array[MeshInstance3D] = []
var torso: MeshInstance3D
var legs: Array[MeshInstance3D] = []
var label: Label3D
var text_timeout := 0.0
var sleeping := false
var model: Node3D
var avatar_version := ""
var is_local := false
var actions: Dictionary = {}
var creator: Node
var audio_allowed := true
var colliders: Array = []

func set_audio_allowed(value: bool) -> void:
	if audio_allowed == value:
		return
	audio_allowed = value
	if model and model.has_meta("ssc_audio"):
		model.get_meta("ssc_audio").set_muted(not audio_allowed)

func _ready() -> void:
	creator = CreatorRuntime.new()
	add_child(creator)
	creator.script_error.connect(runtime_status.emit)
	head = ball(0.13, Color("87d4de"))
	for i in 2:
		hands.append(ball(0.07, Color("e3b3d5")))
		legs.append(ball(0.09, Color("6c9cae")))
	torso = ball(0.23, Color("6c9cae"))
	torso.scale = Vector3(1, 1.8, 0.7)
	label = Label3D.new()
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	label.font_size = 48
	label.pixel_size = 0.002
	label.no_depth_test = false
	add_child(label)

func ball(radius: float, color: Color) -> MeshInstance3D:
	var mesh := MeshInstance3D.new()
	var sphere := SphereMesh.new()
	sphere.radial_segments = 8
	sphere.rings = 4
	sphere.radius = radius
	sphere.height = radius * 2
	mesh.mesh = sphere
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	mesh.material_override = material
	add_child(mesh)
	return mesh

func sample(player: Dictionary, tick: int) -> void:
	if not samples.is_empty() and tick <= int(samples[-1].tick):
		return
	samples.append({"tick": tick, "time": Time.get_ticks_msec() / 1000.0, "pose": player.pose, "tracking":player.get("tracking", {})})
	while samples.size() > 8:
		samples.pop_front()
	sleeping = player.get("sleeping", false)

func _process(delta: float) -> void:
	if is_local or samples.is_empty():
		return
	var target := Time.get_ticks_msec() / 1000.0 - 0.1
	while samples.size() > 2 and float(samples[1].time) < target:
		samples.pop_front()
	var a: Dictionary = samples[0]
	var b: Dictionary = samples[-1] if samples.size() == 1 else samples[1]
	var fraction := clampf((target - float(a.time)) / maxf(0.001, float(b.time) - float(a.time)), 0, 1)
	var transforms: Array[Transform3D] = []
	for i in 3:
		transforms.append(pose_transform(a.pose, i).interpolate_with(pose_transform(b.pose, i), fraction))
	var tracked := BodyTracking.decode(b.get("tracking", {}))
	var previous := BodyTracking.decode(a.get("tracking", {}))
	for name in tracked:
		if previous.has(name):
			tracked[name].transform = previous[name].transform.interpolate_with(tracked[name].transform, fraction)
	_render_pose(delta, transforms, tracked)

func local_pose(delta: float, poses: Array[Transform3D], posture: bool, tracking: Dictionary = {}) -> void:
	is_local = true
	sleeping = posture
	_render_pose(delta, poses, BodyTracking.decode(tracking))
	head.visible = false
	label.visible = false

func _render_pose(delta: float, transforms: Array[Transform3D], tracking: Dictionary = {}) -> void:
	head.transform = transforms[0]
	for i in 2:
		hands[i].transform = transforms[i + 1]
	var h := head.position
	torso.position = h - Vector3(0, 0.45 if not sleeping else 0.12, 0)
	torso.rotation.x = PI / 2 if sleeping else 0.0
	for i in 2:
		legs[i].position = h + Vector3(-0.13 if i == 0 else 0.13, -1.4 if not sleeping else -0.2, 0.8 if sleeping else 0.0)
	label.position = h + Vector3(0, 0.3, 0)
	if model:
		model.update_pose(transforms, sleeping, tracking)
	text_timeout -= delta
	if text_timeout <= 0:
		label.text = ""

func pose_transform(values: Array, index: int) -> Transform3D:
	var i := index * 7
	if values.size() < i + 7:
		return Transform3D.IDENTITY
	var q := Quaternion(float(values[i+3]), float(values[i+4]), float(values[i+5]), float(values[i+6]))
	if q.length_squared() < 0.01:
		q = Quaternion.IDENTITY
	return Transform3D(Basis(q.normalized()), Vector3(float(values[i]), float(values[i+1]), float(values[i+2])))

func chat(text: String) -> void:
	label.text = text
	text_timeout = 10.0

func set_model(root: Node3D, package: String = "", manifest: Dictionary = {}) -> void:
	if creator:
		creator.close()
	if model:
		if model.has_meta("ssc_audio"):
			model.get_meta("ssc_audio").clear()
		model.queue_free()
	model = Humanoid.new()
	add_child(model)
	var queries: bool = manifest.get("scripts", []).any(func(script): return script.get("side") == "client" and "queries" in script.get("capabilities", []))
	model.setup(root, queries)
	colliders = manifest.get("contacts", [])
	if not package.is_empty():
		var audio := PackagedAudio.new()
		model.add_child(audio)
		model.set_meta("ssc_audio", audio)
		audio.set_muted(not audio_allowed)
		audio.failed.connect(runtime_status.emit)
		audio.add_to_group("ssc_packaged_audio")
		audio.load_package(package, manifest.get("sounds", []))
		creator.load_package(package, manifest, model)
	set_actions(actions, true)
	head.visible = false
	torso.visible = false
	for part in hands + legs:
		part.visible = false

func fallback() -> void:
	creator.close()
	colliders.clear()
	if model:
		if model.has_meta("ssc_audio"):
			model.get_meta("ssc_audio").clear()
		model.queue_free()
		model = null
	head.visible = true
	torso.visible = true
	for part in hands + legs:
		part.visible = true

func set_actions(next: Dictionary, force: bool = false) -> void:
	if actions == next and not force:
		return
	actions = next.duplicate()
	for name in actions:
		if model:
			model.creator_action("expression", str(name), float(actions[name]))
		creator.dispatch("action", {"name":name,"weight":actions[name]})

func contact(event: Dictionary) -> void:
	creator.dispatch(str(event.get("action", "touch")), event)
	if model and event.get("action") == "touch":
		model.creator_action("expression", "happy", 0.3)

func grabs(constraints: Array[Dictionary]) -> void:
	if model:
		model.grabs(constraints)
