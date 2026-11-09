extends RefCounted
## Admission bounds retained scenes/workload. Importer peak/RSS and physical GPU
## allocations still require a separate process/allocator-level sandbox.

static func reserve(root: Node, textures_owned: bool = true) -> Dictionary:
	if not ClassDB.class_exists("RenderLease"):
		return {"error":"Renderer budget extension unavailable"}
	var nodes: Array[Node] = [root]
	nodes.append_array(root.find_children("*","Node",true,false))
	if nodes.size() > 4096:
		return {"error":"Imported scene exceeds 4096 nodes"}
	var surfaces := 0
	var vertices := 0
	var texture_bytes := 0
	var bones := 0
	var lights := 0
	var textures: Dictionary = {}
	for node in nodes:
		if node is Skeleton3D:
			bones += node.get_bone_count()
		if node is Light3D:
			lights += 1
		if not node is MeshInstance3D or not node.mesh:
			continue
		for index in node.mesh.get_surface_count():
			# Charge the worst supported tier independently of the user's tier.
			surfaces += 3
			if node.mesh is ArrayMesh:
				vertices += maxi(node.mesh.surface_get_array_len(index),node.mesh.surface_get_array_index_len(index))
			elif node.mesh is PrimitiveMesh:
				vertices += node.mesh.get_mesh_arrays()[Mesh.ARRAY_VERTEX].size()
			else:
				return {"error":"Unsupported mesh type for renderer accounting"}
			if textures_owned:
				var material := node.get_active_material(index) as StandardMaterial3D
				if material:
					for texture in [material.albedo_texture,material.emission_texture,material.normal_texture]:
						if texture and not textures.has(texture.get_instance_id()):
							textures[texture.get_instance_id()] = true
							# RGBA plus mip-chain estimate; count each texture once
							# within this import even when many surfaces reuse it.
							texture_bytes += ceili(float(texture.get_width()) * float(texture.get_height()) * 16.0 / 3.0)
	var lease = ClassDB.instantiate("RenderLease")
	var error: String = lease.reserve(nodes.size(),surfaces,vertices,texture_bytes,bones,lights)
	return {"lease":lease,"nodes":nodes.size(),"surfaces":surfaces,"vertices":vertices,"texture_bytes":texture_bytes,"bones":bones,"lights":lights} if error.is_empty() else {"error":error}
