extends RefCounted
const Locomotion = preload("res://scripts/locomotion.gd")

static func box(parent: Node, center: Vector3, size: Vector3) -> void:
	var body := StaticBody3D.new()
	parent.add_child(body)
	body.position = center
	var collider := CollisionShape3D.new()
	var shape := BoxShape3D.new()
	shape.size = size
	collider.shape = shape
	body.add_child(collider)

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var scene := Node3D.new()
	tree.root.add_child(scene)
	box(scene, Vector3(0, -0.1, 0), Vector3(20, 0.2, 20))
	box(scene, Vector3(1, 1.5, 0), Vector3(0.2, 3, 20))
	var rig := Node3D.new()
	scene.add_child(rig)
	rig.position.y = 1
	var head := Node3D.new()
	rig.add_child(head)
	head.position.y = 1.65
	var body := Locomotion.new()
	body.rig = rig
	body.head = head
	scene.add_child(body)
	body.active = true
	for frame in 60:
		await tree.physics_frame
	assert(absf(rig.position.y) < 0.03, "Gravity must land on the floor")
	body.desired_velocity = Vector3(2, 0, 1)
	for frame in 60:
		await tree.physics_frame
	assert(rig.position.x < 0.73 and rig.position.x > 0.6, "Capsule must stop at walls")
	assert(rig.position.z > 0.7, "Movement must slide along walls")
	assert(absf(rig.position.y) < 0.03, "Walking must stay grounded")
	assert(head.position == Vector3(0, 1.65, 0), "Collisions must never rewrite tracking")
	# Seated/crouched users get a shorter proxy, not a forced head height.
	head.position.y = 0.65
	body.desired_velocity = Vector3.ZERO
	for frame in 3:
		await tree.physics_frame
	assert(is_equal_approx(body.capsule.height, 0.65))
	body.active = false
	var before := rig.transform
	for frame in 3:
		await tree.physics_frame
	assert(rig.transform == before and body.velocity == Vector3.ZERO)
	scene.queue_free()
	await tree.process_frame
	print("PASS local gravity, grounded movement, wall collision/sliding, crouched capsule and untouched tracking")
