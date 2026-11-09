extends VBoxContainer
const RadialMenu = preload("res://scripts/radial_menu.gd")
## Account preferences only. No creator script can change these controls.
signal changed(updates: Dictionary)
var controls: Dictionary = {}
var notice_label: Label

func report(notices: Array[String]) -> void:
	if notice_label:
		notice_label.text = "\n".join(notices)
		notice_label.visible = not notices.is_empty()

static func upscalers(method: String, xr: bool) -> Array:
	# Temporal/stereo upscaling is not assumed to work on untested XR paths.
	return ["bilinear", "fsr1"] if method == "forward_plus" and not xr else ["bilinear"]

func configure(preferences: Dictionary) -> void:
	for child in get_children():
		child.queue_free()
	controls.clear()
	_label("Audio and rendering — account settings")
	_choices("input_device", "Microphone device", Array(AudioServer.get_input_device_list()), preferences)
	_choices("output_device", "Output device", Array(AudioServer.get_output_device_list()), preferences)
	_number("voice_volume", "Voice playback gain", 0, 1, 0.05, preferences, 1)
	_number("microphone_gain", "Microphone gain (does not unmute)", 0, 2, 0.05, preferences, 1)
	_choices("allow_creator_calls", "Allow world interaction scripts to invite/leave calls", [false,true], preferences, ["No (default)","Allow authenticated interactions"])
	_choices("quality", "Material tier", [0,1,2], preferences, ["Basic", "Standard", "Advanced"])
	_number("resolution_scale", "3D resolution scale", 0.5, 1.5, 0.05, preferences, 1)
	_choices("antialiasing", "Antialiasing", ["off","msaa_2x","msaa_4x","msaa_8x"], preferences)
	_choices("upscaler", "Upscaling (unsupported choices fall back)", ["bilinear","fsr1"], preferences)
	_number("texture_lod_bias", "Texture LOD bias (not VRAM reduction)", 0, 3, 0.5, preferences, 0)
	_choices("texture_reduction", "Texture dimensions (next world/avatar load)", [0,1,2,3], preferences, ["Original", "Half", "Quarter", "Eighth"])
	_choices("send_divisor", "Network send divisor (next grant)", [1,2,4], preferences)
	_choices("receive_divisor", "Network receive divisor (next grant)", [1,2,4], preferences)
	_number("radial_hold_seconds", "Quick menu hold time (seconds)", 0.15, 1.5, 0.05, preferences, 0.45)
	var bindings: Array = preferences.get("radial_actions", RadialMenu.DEFAULT_ACTIONS)
	for index in 8:
		var key := "radial_action_" + str(index)
		_choices(key, "Quick menu sector " + str(index + 1), RadialMenu.ACTION_LABELS.keys(), {key:bindings[index]}, RadialMenu.ACTION_LABELS.values())
	var apply := Button.new()
	apply.text = "Apply and save audio/rendering preferences"
	apply.disabled = preferences.is_empty()
	apply.pressed.connect(func():
		var updates: Dictionary = {}
		var actions: Array = []
		for key in controls:
			var control: Control = controls[key]
			if str(key).begins_with("radial_action_"):
				actions.append(control.get_item_metadata(control.selected))
				continue
			if control is OptionButton:
				updates[key] = control.get_item_metadata(control.selected)
			elif control is SpinBox:
				updates[key] = control.value
		updates.radial_actions = actions
		changed.emit(updates)
	)
	add_child(apply)
	notice_label = Label.new()
	notice_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	add_child(notice_label)
	if preferences.is_empty():
		_label("Connect to load and save account settings.")

func _label(text: String) -> void:
	var label := Label.new()
	label.text = text
	add_child(label)

func _choices(key: String, title: String, values: Array, preferences: Dictionary, labels: Array = []) -> void:
	_label(title)
	if values.is_empty():
		values = ["Default"]
	var select := OptionButton.new()
	for index in values.size():
		select.add_item(str(labels[index] if labels.size() == values.size() else values[index]))
		select.set_item_metadata(index, values[index])
	select.select(maxi(0, values.find(preferences.get(key, values[0]))))
	controls[key] = select
	add_child(select)

func _number(key: String, title: String, minimum: float, maximum: float, step: float, preferences: Dictionary, fallback: float) -> void:
	_label(title)
	var spin := SpinBox.new()
	spin.min_value = minimum
	spin.max_value = maximum
	spin.step = step
	spin.value = float(preferences.get(key, fallback))
	controls[key] = spin
	add_child(spin)

static func apply_local(preferences: Dictionary, viewport: Viewport, voice: Node, radio: Node) -> Array[String]:
	var notices: Array[String] = []
	var input := str(preferences.get("input_device", "Default"))
	var output := str(preferences.get("output_device", "Default"))
	var missing_input := not AudioServer.get_input_device_list().has(input)
	if missing_input:
		input = "Default"
		notices.append("Saved microphone unavailable: using Default; microphone remains muted")
		if voice.microphone:
			voice.set_microphone(false)
	if not AudioServer.get_output_device_list().has(output):
		output = "Default"
		notices.append("Saved output device unavailable: using Default")
	if AudioServer.input_device != input:
		var recording: bool = voice.microphone_enabled
		if voice.microphone:
			voice.set_microphone(false)
		AudioServer.input_device = input
		if recording and not missing_input:
			voice.set_microphone(true)
	if AudioServer.output_device != output:
		AudioServer.output_device = output
	voice.playback_gain = clampf(float(preferences.get("voice_volume", 1)), 0, 1)
	if radio.receiver:
		radio.receiver.playback_gain = voice.playback_gain
	if voice.microphone:
		voice.microphone.volume_linear = clampf(float(preferences.get("microphone_gain", 1)), 0, 2)
	var aa := {"off":Viewport.MSAA_DISABLED,"msaa_2x":Viewport.MSAA_2X,"msaa_4x":Viewport.MSAA_4X,"msaa_8x":Viewport.MSAA_8X}
	viewport.msaa_3d = aa.get(str(preferences.get("antialiasing", "msaa_2x")), Viewport.MSAA_DISABLED)
	var scale := clampf(float(preferences.get("resolution_scale", 1)), 0.5, 1.5)
	var upscale := str(preferences.get("upscaler", "bilinear"))
	if not upscalers(RenderingServer.get_current_rendering_method(), viewport.use_xr).has(upscale) or (upscale == "fsr1" and scale > 1):
		notices.append("Requested upscaler unsupported here: using bilinear")
		upscale = "bilinear"
	viewport.scaling_3d_mode = Viewport.SCALING_3D_MODE_FSR if upscale == "fsr1" else Viewport.SCALING_3D_MODE_BILINEAR
	viewport.scaling_3d_scale = scale
	viewport.texture_mipmap_bias = clampf(float(preferences.get("texture_lod_bias", 0)), 0, 3)
	return notices
