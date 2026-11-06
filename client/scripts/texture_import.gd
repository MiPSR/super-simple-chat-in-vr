extends GLTFDocumentExtension
## Decode/resample before GLTFDocument creates GPU textures. Policy is per import,
## never mutable global state shared between world/avatar worker threads.
func _import_preflight(state: GLTFState, _extensions: PackedStringArray) -> Error:
	var reduction = state.get_additional_data(&"ssc/texture_reduction")
	return OK if reduction != null and int(reduction) > 0 else ERR_SKIP

func _parse_image_data(state: GLTFState, data: PackedByteArray, mime: String, image: Image) -> Error:
	var error := ERR_FILE_UNRECOGNIZED
	match mime:
		"image/png": error = image.load_png_from_buffer(data)
		"image/jpeg": error = image.load_jpg_from_buffer(data)
		"image/webp": error = image.load_webp_from_buffer(data)
	if error != OK:
		return error
	var reduction := clampi(int(state.get_additional_data(&"ssc/texture_reduction")), 0, 3)
	image.resize(maxi(1, image.get_width() >> reduction), maxi(1, image.get_height() >> reduction), Image.INTERPOLATE_LANCZOS)
	return OK
