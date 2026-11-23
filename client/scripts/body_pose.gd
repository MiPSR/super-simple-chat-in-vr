extends RefCounted
## Head/controller-only posture estimates. These never move the XR camera.
## Ground comes from authored collision proxies, not an assumed world Y=0.
static func estimate(head: Transform3D, rest_head: Vector3, floors: Dictionary, sleeping: bool, sleep_pose: String) -> Dictionary:
	var forward := -head.basis.z
	forward.y = 0
	var yaw := atan2(-forward.x, -forward.z)
	if forward.length_squared() < 0.0001:
		var right := head.basis.x
		yaw = atan2(-right.z, right.x)
	var upright := Basis(Vector3.UP, yaw)
	var basis := upright
	var height := maxf(0.5, rest_head.y)
	var ground := (float(floors.get("LeftFoot", 0.0)) + float(floors.get("RightFoot", 0.0))) * 0.5
	var relative_height := maxf(0, head.origin.y - ground) / height
	var posture := "standing"
	if sleeping:
		posture = "sleeping_side" if sleep_pose == "side" else "sleeping_flat"
		basis *= Basis(Vector3.RIGHT, PI / 2)
		if sleep_pose == "side":
			basis *= Basis(Vector3.UP, PI / 2)
	elif relative_height < 0.65:
		posture = "sitting"
	elif relative_height < 0.88:
		posture = "crouching"
	var feet: Dictionary = {}
	var knees: Dictionary = {}
	for side in ["Left", "Right"]:
		var name: String = side + "Foot"
		var lateral := (-0.085 if side == "Left" else 0.085) * height
		var reach := -height * 0.28 if posture == "sitting" else 0.0
		var foot := head.origin + basis * Vector3(lateral, -height * 0.91, reach)
		if not sleeping:
			foot.y = float(floors.get(name, ground)) + 0.04
		feet[name] = Transform3D(basis, foot)
		# A stable forward pole prevents head-only knees from flipping backward
		# when a crouched or seated user brings the IK chain near singularity.
		knees[side + "LowerLeg"] = foot + basis * Vector3(0, height * 0.28, -height * 0.18)
	return {"basis":basis, "root":head.origin - basis * rest_head, "feet":feet, "knees":knees, "posture":posture}

static func ground(space: PhysicsDirectSpaceState3D, feet: Dictionary, head: Vector3, height: float) -> Dictionary:
	var floors: Dictionary = {}
	for name in feet:
		var foot: Vector3 = feet[name].origin
		var start := Vector3(foot.x, head.y + 0.2, foot.z)
		var end := Vector3(foot.x, head.y - maxf(3.0, height + 1.0), foot.z)
		var hit := space.intersect_ray(PhysicsRayQueryParameters3D.create(start, end, 1))
		if not hit.is_empty():
			floors[name] = float(hit.position.y)
	# Misses are intentionally absent: a departed platform cannot leave a
	# permanently remembered floor under the avatar.
	return floors
