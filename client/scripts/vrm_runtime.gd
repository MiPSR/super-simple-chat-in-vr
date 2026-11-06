extends RefCounted
## Register only the pinned trusted importer, never uploaded engine resources.
var extensions: Array[GLTFDocumentExtension] = []

class Defaults extends GLTFDocumentExtension:
	func _import_preflight(state: GLTFState, used: PackedStringArray) -> Error:
		if not used.has("VRMC_vrm"):
			return ERR_SKIP
		var doc: Dictionary = state.json
		var vrm: Dictionary = doc.get("extensions", {}).get("VRMC_vrm", {})
		# These optional VRM defaults are indexed directly by the pinned importer.
		var meta: Dictionary = vrm.get("meta", {})
		if not meta.has("modification"):
			meta.modification = "prohibited"
		for group in ["preset", "custom"]:
			for expression in vrm.get("expressions", {}).get(group, {}).values():
				if not expression.has("isBinary"):
					expression.isBinary = false
		state.json = doc
		return OK

func _init() -> void:
	var defaults := Defaults.new()
	extensions.append(defaults)
	extensions.append(preload("res://scripts/texture_import.gd").new())
	for file in ["VRMC_vrm", "VRMC_node_constraint", "VRMC_springBone", "VRMC_materials_hdr_emissiveMultiplier", "VRMC_materials_mtoon"]:
		var path := "res://addons/vrm/1.0/%s.gd" % file
		if not ResourceLoader.exists(path):
			continue
		var script: Script = load(path)
		if script:
			var extension: GLTFDocumentExtension = script.new()
			extensions.append(extension)
	# Runtime Godot registers its ImporterMesh -> MeshInstance converter already.
	# VRM needs the import-stage nodes, so its hooks must precede that converter.
	for index in range(extensions.size() - 1, -1, -1):
		GLTFDocument.register_gltf_document_extension(extensions[index], true)

func available() -> bool:
	return extensions.size() > 2

func close() -> void:
	for extension in extensions:
		GLTFDocument.unregister_gltf_document_extension(extension)
	extensions.clear()
