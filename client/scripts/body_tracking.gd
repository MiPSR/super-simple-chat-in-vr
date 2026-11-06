extends RefCounted
## Only meaningful body inputs travel over the wire, never an entire animated rig.
const JOINTS := {
	"Hips":XRBodyTracker.JOINT_HIPS,
	"Chest":XRBodyTracker.JOINT_CHEST,
	"LeftLowerLeg":XRBodyTracker.JOINT_LEFT_LOWER_LEG,
	"RightLowerLeg":XRBodyTracker.JOINT_RIGHT_LOWER_LEG,
	"LeftFoot":XRBodyTracker.JOINT_LEFT_FOOT,
	"RightFoot":XRBodyTracker.JOINT_RIGHT_FOOT,
}

func collect(origin: Transform3D) -> Dictionary:
	for tracker in XRServer.get_trackers(XRServer.TRACKER_BODY).values():
		if tracker is XRBodyTracker and tracker.has_tracking_data:
			return from_tracker(tracker, origin)
	return {}

func from_tracker(tracker: XRBodyTracker, origin: Transform3D) -> Dictionary:
	var result: Dictionary = {}
	if not tracker.has_tracking_data:
		return result
	var valid := XRBodyTracker.JOINT_FLAG_POSITION_VALID | XRBodyTracker.JOINT_FLAG_ORIENTATION_VALID
	var tracked := XRBodyTracker.JOINT_FLAG_POSITION_TRACKED | XRBodyTracker.JOINT_FLAG_ORIENTATION_TRACKED
	for name in JOINTS:
		var flags := tracker.get_joint_flags(JOINTS[name])
		if flags & valid != valid:
			continue
		var pose := origin * XRServer.get_reference_frame() * tracker.get_joint_transform(JOINTS[name])
		if not pose.is_finite():
			continue
		var q := pose.basis.orthonormalized().get_rotation_quaternion()
		result[name] = {"pose":[pose.origin.x,pose.origin.y,pose.origin.z,q.x,q.y,q.z,q.w], "source":"tracked" if flags & tracked == tracked else "estimated"}
	return result

static func decode(data: Dictionary) -> Dictionary:
	var result: Dictionary = {}
	for name in data:
		if not JOINTS.has(name) or not data[name] is Dictionary:
			continue
		var values: Array = data[name].get("pose", [])
		if values.size() != 7:
			continue
		var q := Quaternion(float(values[3]),float(values[4]),float(values[5]),float(values[6]))
		if q.length_squared() < 0.01:
			continue
		result[name] = {"transform":Transform3D(Basis(q.normalized()),Vector3(float(values[0]),float(values[1]),float(values[2]))), "source":str(data[name].get("source","tracked"))}
	return result
