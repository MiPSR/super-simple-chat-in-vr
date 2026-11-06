extends Node3D
## Presentation follows bounded received samples; gameplay physics stays on Rust.
const Materials = preload("res://scripts/materials.gd")
var samples: Array[Dictionary] = []
var visual: Node3D
var health: Label3D
var prefab_id := ""
var descriptor: Dictionary = {}
var collision: CollisionShape3D

func configure(data: Dictionary, prefabs: Dictionary, quality: int, tick: int) -> void:
	descriptor = data
	var next_prefab := str(data.get("prefab", ""))
	if not visual or prefab_id != next_prefab or (visual.has_meta("fallback") and prefabs.has(next_prefab)):
		if visual:
			visual.queue_free()
		prefab_id = next_prefab
		if prefabs.has(prefab_id):
			visual = prefabs[prefab_id].instantiate()
		else:
			var mesh := MeshInstance3D.new()
			var box := BoxMesh.new()
			var size: Array = data.get("size", [0.25, 0.25, 0.25])
			box.size = Vector3(float(size[0]), float(size[1]), float(size[2]))
			mesh.mesh = box
			visual = mesh
			visual.set_meta("fallback", true)
		add_child(visual)
		Materials.apply(visual, quality)
	visible = bool(data.get("visible", true)) and not (data.get("component", "") == "door" and bool(data.get("active", false)))
	if not health:
		health = Label3D.new()
		health.billboard = BaseMaterial3D.BILLBOARD_ENABLED
		health.font_size = 36
		health.pixel_size = 0.002
		add_child(health)
	health.visible = float(data.get("health", 0)) > 0
	health.text = "%s HP" % int(data.get("health", 0))
	health.position.y = float(data.get("size", [0.25, 0.25, 0.25])[1]) / 2 + 0.3
	if not collision and data.get("component", "") in ["static", "door", "vehicle", "prop"]:
		var body := StaticBody3D.new()
		add_child(body)
		collision = CollisionShape3D.new()
		var shape := BoxShape3D.new()
		var size: Array = data.get("size", [0.25,0.25,0.25])
		shape.size = Vector3(float(size[0]),float(size[1]),float(size[2]))
		collision.shape = shape
		body.add_child(collision)
	if collision:
		collision.set_deferred("disabled", data.get("component", "") == "door" and bool(data.get("active", false)))
	sample(data, tick)

func sample(data: Dictionary, tick: int) -> void:
	if not samples.is_empty() and tick <= int(samples[-1].tick):
		return
	var p: Array = data.position
	var r: Array = data.get("rotation", [0, 0, 0, 1])
	var q := Quaternion(float(r[0]), float(r[1]), float(r[2]), float(r[3])).normalized()
	var pose := Transform3D(Basis(q), Vector3(float(p[0]), float(p[1]), float(p[2])))
	if samples.is_empty():
		transform = pose
	samples.append({"tick":tick, "time":Time.get_ticks_msec() / 1000.0, "pose":pose})
	while samples.size() > 8:
		samples.pop_front()

func _process(_delta: float) -> void:
	if samples.is_empty():
		return
	var target := Time.get_ticks_msec() / 1000.0 - 0.1
	while samples.size() > 2 and float(samples[1].time) < target:
		samples.pop_front()
	var a: Dictionary = samples[0]
	var b: Dictionary = samples[1] if samples.size() > 1 else a
	var weight := clampf((target - float(a.time)) / maxf(0.001, float(b.time) - float(a.time)), 0, 1)
	transform = a.pose.interpolate_with(b.pose, weight)
