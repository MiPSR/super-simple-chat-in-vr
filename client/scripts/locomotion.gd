extends CharacterBody3D
## Collide artificial movement, never push the physical headset or consume
## network corrections. Room-scale head/hand tracking stays immediate.
var rig: Node3D
var head: Node3D
var desired_velocity := Vector3.ZERO
var active := false
var capsule := CapsuleShape3D.new()

func _ready() -> void:
	collision_layer = 0
	collision_mask = 1
	floor_snap_length = 0.2
	floor_stop_on_slope = true
	capsule.radius = 0.2
	capsule.height = 1.65
	var collider := CollisionShape3D.new()
	collider.shape = capsule
	add_child(collider)

func reset() -> void:
	velocity = Vector3.ZERO
	desired_velocity = Vector3.ZERO

func _physics_process(delta: float) -> void:
	if not active or not is_instance_valid(rig) or not is_instance_valid(head):
		reset()
		return
	# Reposition only the proxy to the tracked head; feet use the rig floor.
	var height := clampf(head.global_position.y - rig.global_position.y, 0.45, 2.5)
	capsule.height = height
	global_position = Vector3(head.global_position.x, rig.global_position.y + height * 0.5, head.global_position.z)
	var before := global_position
	velocity.x = desired_velocity.x
	velocity.z = desired_velocity.z
	if is_on_floor() and velocity.y < 0:
		velocity.y = 0
	else:
		velocity.y = maxf(velocity.y - 9.8 * delta, -30.0)
	move_and_slide()
	rig.global_position += global_position - before
