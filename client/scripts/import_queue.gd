extends RefCounted
## One active GLTF decode/construction per client, with FIFO weak waiters.
## This bounds concurrent peaks, not the memory used by an individual importer.
static var owner: WeakRef
static var waiters: Array[WeakRef] = []

static func request(node: Node) -> bool:
	if owner and owner.get_ref() == node:
		return true
	waiters = waiters.filter(func(reference: WeakRef): return is_instance_valid(reference.get_ref()))
	if not waiters.any(func(reference: WeakRef): return reference.get_ref() == node):
		waiters.append(weakref(node))
	if owner and is_instance_valid(owner.get_ref()):
		return false
	if waiters[0].get_ref() != node:
		return false
	waiters.pop_front()
	owner = weakref(node)
	return true

static func release(node: Node) -> void:
	if owner and owner.get_ref() == node:
		owner = null
	waiters = waiters.filter(func(reference: WeakRef): return is_instance_valid(reference.get_ref()) and reference.get_ref() != node)
