extends Node3D
const Network = preload("res://scripts/network.gd")
const Avatar = preload("res://scripts/avatar.gd")
const Loader = preload("res://scripts/package_loader.gd")
const Materials = preload("res://scripts/materials.gd")
const Entity = preload("res://scripts/entity.gd")
const VRMRuntime = preload("res://scripts/vrm_runtime.gd")
const CreatorRuntime = preload("res://scripts/creator_runtime.gd")
const Voice = preload("res://scripts/voice.gd")
const BodyTracking = preload("res://scripts/body_tracking.gd")
const VideoSurface = preload("res://scripts/video_surface.gd")
const PackagedAudio = preload("res://scripts/packaged_audio.gd")
const SettingsPanel = preload("res://scripts/settings_panel.gd")
const Acoustics = preload("res://scripts/acoustics.gd")
const RadialMenu = preload("res://scripts/radial_menu.gd")
const Radio = preload("res://scripts/radio.gd")
const ContactEvents = preload("res://scripts/contact_events.gd")
const AmbientVisuals = preload("res://scripts/ambient_visuals.gd")
var ambient_visuals := AmbientVisuals.new()
var network: Node
var loader: Node
var origin: XROrigin3D
var camera: Camera3D
var controllers: Array[XRController3D] = []
var avatars: Dictionary = {}
var world_root: Node3D
var current_version := ""
var current_instance := ""
var phone: Node3D
var phone_viewport: SubViewport
var desktop: SubViewportContainer
var grant_input: TextEdit
var chat_input: LineEdit
var status_label: Label
var radial: Control
var phone_before_radial := false
var phone_hand := 0
var quality := 1
var xr_active := false
var menu_held := 0.0
var menu_hold_delay := 0.45
var menu_pressed := false
var sleeping := false
var entity_nodes: Dictionary = {}
var entity_data: Dictionary = {}
var selected_entity := ""
var drive_elapsed := 0.0
var latest_tick := 0
var vrm_runtime: RefCounted
var avatar_queue: Array[Dictionary] = []
var avatar_loader: Node
var avatar_request: HTTPRequest
var avatar_choices: OptionButton
var creator: Node
var voice: Node3D
var avatar_action_input: LineEdit
var body_tracking := BodyTracking.new()
var interactions: Dictionary = {}
var contact_events := ContactEvents.new()
var preferences: Dictionary = {}
var preferences_request: HTTPRequest
var touch_elapsed := 0.0
var call_user: LineEdit
var last_call_invitation := ""
var video_surfaces: Dictionary = {}
var media_states: Dictionary = {}
var media_choices: OptionButton
var media_url: LineEdit
var media_seek: SpinBox
var acoustics: Node
var audio_effect_max := 0.9
var radios: Node3D
var radio_choices: OptionButton
var radio_text: LineEdit
var radio_image_dialog: FileDialog
var radio_policy: Dictionary = {}
var radio_preset := "standard"
var radio_bitrate_ceiling := 8000
var radio_presets_ui: OptionButton
var settings_panel: VBoxContainer
var preference_save_timer: Timer
var preference_save_pending := false
var input_focused := true
var phone_before_focus := false
var xr_interface: OpenXRInterface

func _update_xr_focus() -> void:
	if xr_interface:
		var focused := xr_interface.get_session_state() == OpenXRInterface.SESSION_STATE_FOCUSED
		if xr_interface.is_user_presence_supported():
			focused = focused and xr_interface.is_user_present()
		_set_input_focus(focused)

func _notification(what: int) -> void:
	if not xr_active and what == NOTIFICATION_APPLICATION_FOCUS_OUT:
		_set_input_focus(false)
	elif not xr_active and what == NOTIFICATION_APPLICATION_FOCUS_IN:
		_set_input_focus(true)

func _set_input_focus(focused: bool) -> void:
	if input_focused == focused:
		return
	input_focused = focused
	if is_instance_valid(voice):
		voice.set_input_focus(focused)
	if is_instance_valid(network):
		network.set_input_focus(focused)
	if not focused:
		selected_entity = ""
		menu_pressed = false
		menu_held = 0
		drive_elapsed = 0
		if is_instance_valid(radial):
			radial.visible = false
		if is_instance_valid(phone):
			phone_before_focus = phone.visible
			phone.visible = false
	elif is_instance_valid(phone):
		phone.visible = phone_before_focus

func _ready() -> void:
	vrm_runtime = VRMRuntime.new()
	network = Network.new()
	add_child(network)
	network.message.connect(_message)
	network.status.connect(_status)
	network.disconnected.connect(_clear_instance)
	voice = Voice.new()
	add_child(voice)
	voice.attach(network)
	voice.status.connect(_status)
	radios = Radio.new()
	add_child(radios)
	radios.attach(network)
	radios.status.connect(_status)
	acoustics = Acoustics.new()
	add_child(acoustics)
	loader = Loader.new()
	add_child(loader)
	creator = CreatorRuntime.new()
	add_child(creator)
	creator.script_error.connect(_status)
	loader.failed.connect(_status)
	loader.cache_notice.connect(_status)
	loader.loaded.connect(func(root: Node3D):
		if world_root:
			if world_root.has_meta("ssc_audio"):
				world_root.get_meta("ssc_audio").clear()
			world_root.queue_free()
		world_root = root
		add_child(root)
		var audio := PackagedAudio.new()
		root.add_child(audio)
		root.set_meta("ssc_audio", audio)
		root.set_meta("ssc_radio", radios)
		root.set_meta("ssc_network", network)
		root.set_meta("ssc_entities", entity_nodes)
		root.set_meta("ssc_visual_host",self)
		root.set_meta("ssc_acoustics",acoustics)
		audio.add_to_group("ssc_packaged_audio")
		audio.failed.connect(_status)
		audio.load_package(loader.staging, loader.manifest.get("sounds", []))
		for id in entity_data:
			entity_nodes[id].configure(entity_data[id], loader.prefabs, quality, latest_tick)
		acoustics.configure(loader.manifest.get("audio_zones", []), audio_effect_max,bool(loader.manifest.get("realistic_acoustics",false)))
		creator.load_package(loader.staging, loader.manifest, root, true)
		_setup_video(loader.manifest)
		radios.configure(loader.manifest.get("radios", []),radio_policy)
		radio_choices.clear()
		for spec in loader.manifest.get("radios", []):
			radio_choices.add_item(str(spec.id) + " · ch " + str(spec.channel))
			radio_choices.set_item_metadata(radio_choices.item_count - 1, str(spec.id))
	)
	_build_rig()
	_build_lobby()
	_build_phone()
	voice.set_input_focus(input_focused)
	network.set_input_focus(input_focused)
	if not input_focused:
		phone_before_focus = phone.visible
		phone.visible = false
	_status("Open the website, sign in, select a world, and paste a one-use grant below.")

func _build_rig() -> void:
	origin = XROrigin3D.new()
	add_child(origin)
	var xr := XRServer.find_interface("OpenXR") as OpenXRInterface
	if xr and xr.initialize():
		xr_active = true
		xr_interface = xr
		xr.session_focussed.connect(_update_xr_focus)
		xr.session_visible.connect(func(): _set_input_focus(false))
		xr.session_stopping.connect(func(): _set_input_focus(false))
		xr.session_synchronized.connect(func(): _set_input_focus(false))
		xr.session_loss_pending.connect(func(): _set_input_focus(false))
		xr.instance_exiting.connect(func(): _set_input_focus(false))
		xr.user_presence_changed.connect(func(_present: bool): _update_xr_focus())
		_update_xr_focus()
		get_viewport().use_xr = true
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
		camera = XRCamera3D.new()
	else:
		camera = Camera3D.new()
		camera.position = Vector3(0, 1.65, 3)
		camera.current = true
	origin.add_child(camera)
	for i in 2:
		var controller := XRController3D.new()
		controller.tracker = &"left_hand" if i == 0 else &"right_hand"
		origin.add_child(controller)
		controllers.append(controller)
		controller.button_pressed.connect(_controller_pressed)
		controller.button_released.connect(_controller_released)

func _build_lobby() -> void:
	var floor_mesh := MeshInstance3D.new()
	var box := BoxMesh.new()
	box.size = Vector3(20, 0.1, 20)
	floor_mesh.mesh = box
	floor_mesh.position.y = -0.08
	add_child(floor_mesh)
	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-50, -30, 0)
	add_child(light)
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color("101622")
	environment.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.environment.ambient_light_color = Color.WHITE
	environment.environment.ambient_light_energy = 0.5
	add_child(environment)

func _button(parent: Node, text: String, callback: Callable) -> Button:
	var button := Button.new()
	button.text = text
	button.custom_minimum_size.y = 54
	button.pressed.connect(callback)
	parent.add_child(button)
	return button

func _build_phone() -> void:
	phone_viewport = SubViewport.new()
	phone_viewport.size = Vector2i(560, 960)
	phone_viewport.disable_3d = true
	phone_viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	phone_viewport.gui_disable_input = false
	var layer := CanvasLayer.new()
	add_child(layer)
	desktop = SubViewportContainer.new()
	desktop.size = Vector2(560, 960)
	desktop.scale = Vector2(0.78, 0.78)
	desktop.visible = not xr_active
	layer.add_child(desktop)
	desktop.add_child(phone_viewport)
	var panel := PanelContainer.new()
	panel.size = Vector2(560, 960)
	phone_viewport.add_child(panel)
	var scroll := ScrollContainer.new()
	scroll.custom_minimum_size = Vector2(540, 920)
	panel.add_child(scroll)
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 12)
	column.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(column)
	var title := Label.new()
	title.text = "Super Simple Chat in VR"
	title.add_theme_font_size_override("font_size", 29)
	column.add_child(title)
	status_label = Label.new()
	status_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	status_label.custom_minimum_size = Vector2(520, 100)
	column.add_child(status_label)
	grant_input = TextEdit.new()
	grant_input.placeholder_text = "Paste one-use connection grant JSON"
	grant_input.custom_minimum_size = Vector2(520, 160)
	column.add_child(grant_input)
	_button(column, "Connect", _connect)
	_button(column, "Toggle microphone mute", func(): voice.set_microphone(not voice.microphone_enabled))
	radio_choices = OptionButton.new()
	column.add_child(radio_choices)
	radio_presets_ui = OptionButton.new()
	for label in ["Radio: standard (admin ceiling)", "Radio: narrow", "Radio: weak signal"]:
		radio_presets_ui.add_item(label)
	radio_presets_ui.item_selected.connect(func(index: int):
		radio_preset = ["standard", "narrow", "weak"][index]
		_apply_radio_preset()
		if not preferences.is_empty():
			preferences.radio_preset = radio_preset
			_save_preferences()
	)
	column.add_child(radio_presets_ui)
	_button(column, "Route microphone: selected radio / proximity", func():
		if radio_choices.selected >= 0:
			voice.radio_target = str(radio_choices.get_item_metadata(radio_choices.selected)) if voice.radio_target.is_empty() else ""
			_status("Microphone route: " + (voice.radio_target if not voice.radio_target.is_empty() else "proximity / call"))
	)
	radio_text = LineEdit.new()
	radio_text.placeholder_text = "FT8-style message (40 ASCII characters)"
	radio_text.max_length = 40
	column.add_child(radio_text)
	_button(column, "Transmit FT8-style message", func(): _radio_transmit("ft8", radio_text.text.to_upper()))
	radio_image_dialog = FileDialog.new()
	radio_image_dialog.access = FileDialog.ACCESS_FILESYSTEM
	radio_image_dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	radio_image_dialog.filters = PackedStringArray(["*.png,*.jpg,*.jpeg; SSTV source image"])
	phone_viewport.add_child(radio_image_dialog)
	radio_image_dialog.file_selected.connect(func(path: String):
		var file := FileAccess.open(path,FileAccess.READ)
		if not file or file.get_length() > 8000000:
			_status("SSTV image exceeds the 8 MB input budget")
			return
		var image := Image.load_from_file(path)
		if image and not image.is_empty():
			image.resize(64,48,Image.INTERPOLATE_LANCZOS)
			image.convert(Image.FORMAT_L8)
			_radio_transmit("sstv", image.get_data().hex_encode())
	)
	_button(column, "Transmit SSTV image", func(): radio_image_dialog.popup_centered(Vector2i(520,700)))
	media_choices = OptionButton.new()
	column.add_child(media_choices)
	media_url = LineEdit.new()
	media_url.placeholder_text = "Public video or YouTube URL"
	column.add_child(media_url)
	for action in ["load", "play", "pause", "stop"]:
		_button(column, "Video: " + action, _media_command.bind(action))
	media_seek = SpinBox.new()
	media_seek.max_value = 604800
	media_seek.suffix = " seconds"
	column.add_child(media_seek)
	_button(column, "Video: seek", _media_command.bind("seek"))
	call_user = LineEdit.new()
	call_user.placeholder_text = "Discord ID to call"
	column.add_child(call_user)
	_button(column, "Invite to private call", func(): network.command(21, {"action":"invite", "user":call_user.text}))
	_button(column, "Accept latest call invitation", func():
		if not last_call_invitation.is_empty():
			network.command(21, {"action":"accept", "call":last_call_invitation})
	)
	_button(column, "Leave private call", func(): network.command(21, {"action":"leave"}))
	_button(column, "Cycle call routing preference", func():
		if preferences.is_empty():
			return
		var modes := ["automatic", "private", "spatial"]
		preferences.call_mode = modes[(modes.find(preferences.get("call_mode", "automatic")) + 1) % 3]
		_save_preferences()
		_status("Call routing: " + str(preferences.call_mode))
	)
	chat_input = LineEdit.new()
	chat_input.placeholder_text = "Text above your head (Enter to send)"
	chat_input.max_length = 256
	column.add_child(chat_input)
	chat_input.text_submitted.connect(func(text: String): network.command(4, {"text": text}); chat_input.clear())
	_button(column, "Interact with nearest switch / door", _interact)
	_button(column, "Claim nearest vehicle", func(): _claim("vehicle"))
	_button(column, "Grab nearest prop", func(): _claim("prop"))
	_button(column, "Fire selected vehicle", _fire)
	_button(column, "Release object / vehicle", _release)
	avatar_choices = OptionButton.new()
	avatar_choices.add_item("Fallback avatar")
	avatar_choices.item_selected.connect(func(index: int): network.command(8, {"avatar":avatar_choices.get_item_metadata(index)}))
	column.add_child(avatar_choices)
	avatar_request = HTTPRequest.new()
	avatar_request.body_size_limit = 1_000_000
	add_child(avatar_request)
	avatar_request.request_completed.connect(_avatar_catalog)
	preferences_request = HTTPRequest.new()
	add_child(preferences_request)
	preferences_request.request_completed.connect(_preferences_received)
	preference_save_timer = Timer.new()
	preference_save_timer.one_shot = true
	preference_save_timer.wait_time = 0.25
	preference_save_timer.timeout.connect(_flush_preferences)
	add_child(preference_save_timer)
	network.negotiated.connect(func(_bitrate: int): preferences_request.request(loader.base_url + "/api/v1/me", PackedStringArray(["Authorization: Bearer " + loader.credentials])))
	_button(column, "Toggle my touch permissions", func():
		if not preferences.is_empty():
			preferences.can_touch_me = not bool(preferences.can_touch_me)
			preferences.can_touch_others = preferences.can_touch_me
			_save_preferences()
	)
	_button(column, "Head pat nearest avatar", func(): _contact_nearest("touch"))
	_button(column, "Grab nearest hair / accessory", func(): _contact_nearest("grab"))
	_button(column, "Release avatar grab", func():
		for contact in interactions.values():
			if contact.actor == network.user:
				network.command(6, {"action":"release", "target":contact.target, "hand":contact.hand})
	)
	_button(column, "Refresh my avatar slots", func():
		if network.joined:
			avatar_request.request(loader.base_url + "/api/v1/assets?kind=avatar", PackedStringArray(["Authorization: Bearer " + loader.credentials]))
	)
	avatar_action_input = LineEdit.new()
	avatar_action_input.placeholder_text = "Authored avatar action name"
	column.add_child(avatar_action_input)
	_button(column, "Apply avatar action", func(): network.command(13, {"name":avatar_action_input.text, "weight":1.0}))
	_button(column, "Reset avatar action", func(): network.command(13, {"name":avatar_action_input.text, "weight":0.0}))
	_button(column, "Switch phone hand", _switch_phone_hand)
	_button(column, "Toggle sleeping pose", _sleep)
	settings_panel = SettingsPanel.new()
	column.add_child(settings_panel)
	settings_panel.configure(preferences)
	settings_panel.changed.connect(func(updates: Dictionary):
		if preferences.is_empty():
			return
		# Merge only controlled fields: stale UI cannot re-enable touch permissions.
		preferences.merge(updates, true)
		_apply_preferences()
		_save_preferences()
	)
	_button(column, "Disconnect", func(): network.close(); _clear_instance(); _status("Disconnected"))
	radial = RadialMenu.new()
	radial.position = Vector2(20,220)
	radial.size = Vector2(520,520)
	radial.mouse_filter = Control.MOUSE_FILTER_IGNORE
	radial.visible = false
	phone_viewport.add_child(radial)
	phone = Node3D.new()
	add_child(phone)
	var surface := MeshInstance3D.new()
	var quad := QuadMesh.new()
	quad.size = Vector2(0.28, 0.48)
	surface.mesh = quad
	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.albedo_texture = phone_viewport.get_texture()
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	surface.material_override = mat
	phone.add_child(surface)
	phone.visible = xr_active

func _connect() -> void:
	var grant = JSON.parse_string(grant_input.text)
	if not grant is Dictionary:
		_status("Invalid grant JSON")
		return
	var api_url := str(grant.get("api_url", ""))
	if not api_url.begins_with("https://") and not api_url.begins_with("http://127.0.0.1:"):
		_status("Asset API requires HTTPS (except loopback development)")
		return
	preferences_request.cancel_request()
	preference_save_timer.stop()
	preference_save_pending = false
	preferences = {}
	settings_panel.configure(preferences)
	loader.credentials = str(grant.get("api_token", ""))
	loader.base_url = api_url
	_clear_instance()
	var result: Error = network.connect_grant(grant)
	grant_input.text = ""
	if result != OK:
		_status(error_string(result))

func _clear_instance() -> void:
	ambient_visuals.clear()
	loader.cancel()
	for avatar in avatars.values():
		avatar.fallback()
		avatar.queue_free()
	avatars.clear()
	for entity in entity_nodes.values():
		entity.queue_free()
	entity_nodes.clear()
	entity_data.clear()
	for surface in video_surfaces.values():
		surface.close()
		surface.queue_free()
	video_surfaces.clear()
	media_states.clear()
	acoustics.clear()
	radios.clear()
	interactions.clear()
	contact_events.clear()
	selected_entity = ""
	loader.prefabs.clear()
	creator.close()
	voice.clear()
	avatar_queue.clear()
	if avatar_loader:
		avatar_loader.retire()
		avatar_loader = null
	if world_root:
		if world_root.has_meta("ssc_audio"):
			world_root.get_meta("ssc_audio").clear()
		world_root.queue_free()
		world_root = null
	current_version = ""
	current_instance = ""

func _message(kind: int, data: Dictionary) -> void:
	if kind == 2 and data.has("user"):
		radios.bind_actor(str(data.user))
	if data.has("audio_effect_max"):
		audio_effect_max = clampf(float(data.audio_effect_max), 0, 1)
	if data.has("radio_bitrate"):
		radio_bitrate_ceiling = int(data.radio_bitrate)
		radio_policy = data.get("radio_policy", {})
		_apply_radio_preset()
	if kind == 255:
		_status(str(data.get("error", "Server error")))
	elif kind == 2:
		var snapshot: Dictionary = data.snapshot
		if current_instance != str(snapshot.instance):
			_clear_instance()
			current_instance = str(snapshot.instance)
		if current_version != str(snapshot.version):
			current_version = str(snapshot.version)
			loader.quality = quality
			loader.load_world(snapshot)
			var spawn: Array = snapshot.manifest.spawn
			origin.position = Vector3(float(spawn[0]), float(spawn[1]), float(spawn[2]))
		_update_players(snapshot.players.values(), int(snapshot.tick), true)
		latest_tick = int(snapshot.tick)
		entity_data = snapshot.entities
		creator.world_queries = snapshot.manifest.get("scripts", []).any(func(s): return s.get("side") == "client")
		creator.update_objects(entity_data, int(snapshot.tick))
		creator.update_state(snapshot.get("variables", {}), int(snapshot.tick))
		var next_contacts: Dictionary = snapshot.get("interactions", {})
		for event in contact_events.replace(next_contacts):
			event.snapshot_tick = int(snapshot.tick)
			if avatars.has(str(event.target)):
				avatars[str(event.target)].contact(event)
			creator.dispatch("release", event)
		interactions = next_contacts
		_update_media(snapshot.get("media", {}))
		creator.dispatch("state", snapshot.get("variables", {}))
		for id in entity_nodes.keys():
			if not entity_data.has(id):
				entity_nodes[id].queue_free()
				entity_nodes.erase(id)
		for id in entity_data:
			_upsert_entity(entity_data[id], latest_tick)
	elif kind == 3:
		creator.update_transforms(data.get("entities", []), int(data.tick))
		_update_media(data.get("media", {}))
		_update_players(data.players, int(data.tick), false)
		for entity in data.get("entities", []):
			if entity_nodes.has(str(entity.id)):
				entity_nodes[str(entity.id)].sample(entity, int(data.tick))
	elif kind == 4:
		if avatars.has(str(data.user)):
			avatars[str(data.user)].chat(str(data.text))
		_status("%s: %s" % [data.user, data.text])
	elif kind == 6:
		contact_events.acknowledge(data)
		if avatars.has(str(data.target)):
			avatars[str(data.target)].contact(data)
		creator.dispatch(str(data.get("action", "touch")), data)
	elif kind == 21:
		if data.get("event") == "invite":
			last_call_invitation = str(data.call)
			_status("Private call invitation from " + str(data.from))
		else:
			voice.call_state(data.get("calls", []))
			_status("Call status: " + JSON.stringify(data.get("calls", [])))
	elif kind == 24 or kind == 25:
		if kind == 24:
			voice.prefer_radio(str(data.sender))
		radios.receive(data)
	elif kind == 11:
		var graph_changed := false
		var event_tick := int(data.tick)
		for event in data.events:
			if event.event == "sound" and world_root and world_root.has_meta("ssc_audio"):
				var audio: Node = world_root.get_meta("ssc_audio")
				match str(event.action):
					"play_sound": audio.play_sound(str(event.target))
					"stop_sound": audio.stop_sound(str(event.target))
					"seek_sound": audio.seek_sound(str(event.target), float(event.value))
			elif event.event == "spawn" and event_tick > latest_tick:
				entity_data[str(event.entity.id)] = event.entity
				_upsert_entity(event.entity, int(data.tick))
				graph_changed = true
			elif event.event == "destroy" and event_tick > latest_tick:
				var id := str(event.target)
				if entity_nodes.has(id):
					entity_nodes[id].queue_free()
					entity_nodes.erase(id)
				entity_data.erase(id)
				graph_changed = true
		# A reliable graph for this tick precedes its notifications. Spawn data
		# describes creation-time poses, not necessarily the final tick pose.
		if graph_changed:
			creator.update_objects(entity_data, event_tick)
		creator.queue_world_events(data.events)

func _upsert_entity(data: Dictionary, tick: int) -> void:
	var id := str(data.id)
	if not entity_nodes.has(id):
		var entity := Entity.new()
		entity.presentation_error.connect(_status)
		add_child(entity)
		entity_nodes[id] = entity
	entity_nodes[id].configure(data, loader.prefabs, quality, tick)

func _setup_video(manifest: Dictionary) -> void:
	for surface in video_surfaces.values():
		surface.close()
		surface.queue_free()
	video_surfaces.clear()
	media_choices.clear()
	for spec in manifest.get("video_surfaces", []):
		var surface := VideoSurface.new()
		add_child(surface)
		surface.failed.connect(_status)
		surface.setup(spec)
		video_surfaces[str(spec.id)] = surface
		media_choices.add_item(str(spec.id))
	_update_media(media_states)

func _update_media(states: Dictionary) -> void:
	for id in states:
		if float(states[id].get("sampled_at", 0)) < float(media_states.get(id, {}).get("sampled_at", 0)):
			continue
		media_states[id] = states[id]
		if video_surfaces.has(id):
			video_surfaces[id].update_timeline(network.media_timeline(states[id]))

func _media_command(action: String) -> void:
	if media_choices.selected < 0:
		return
	network.command(22, {"target":media_choices.get_item_text(media_choices.selected), "action":action, "source":media_url.text, "value":media_seek.value})

func _radio_transmit(mode: String, data: String) -> void:
	if radio_choices.selected >= 0:
		if not radios.transmit(str(radio_choices.get_item_metadata(radio_choices.selected)), mode, data):
			_status("Radio request not queued: check connection, payload and cooldown")

func _apply_radio_preset() -> void:
	var allowed: Array = radio_policy.get("presets", ["standard", "narrow", "weak"])
	if not allowed.has(radio_preset):
		radio_preset = str(allowed[0])
	radios.allowed_presets = allowed
	var names := ["standard", "narrow", "weak"]
	for index in names.size():
		radio_presets_ui.set_item_disabled(index, not allowed.has(names[index]))
	radio_presets_ui.select(names.find(radio_preset))
	radios.apply_preset(radio_preset)
	var factor := 1.0 if radio_preset == "standard" else 0.75 if radio_preset == "narrow" else 0.5
	voice.configure_radio(clampi(int(radio_bitrate_ceiling * factor), 8000, radio_bitrate_ceiling))

func _update_players(players: Array, tick: int, complete: bool) -> void:
	voice.positions(players,complete)
	var seen: Dictionary = {}
	for player in players:
		var id := str(player.id)
		seen[id] = true
		if not avatars.has(id):
			var avatar := Avatar.new()
			avatar.runtime_status.connect(_status)
			add_child(avatar)
			avatars[id] = avatar
			Materials.apply(avatar, quality)
		avatars[id].set_audio_allowed(bool(player.get("audio_allowed", id == network.user)))
		if id == network.user:
			avatars[id].is_local = true
		else:
			avatars[id].sample(player, tick)
		avatars[id].set_actions(player.get("avatar_actions", {}))
		var version := str(player.get("avatar_version", ""))
		if player.get("avatar_version") == null:
			version = ""
		if avatars[id].avatar_version != version:
			avatars[id].avatar_version = version
			avatars[id].fallback()
			if not version.is_empty() and vrm_runtime.available():
				avatar_queue.append({"player":id, "asset":str(player.avatar), "version":version})
	if complete:
		for id in avatars.keys():
			if not seen.has(id):
				avatars[id].fallback()
				avatars[id].queue_free()
				avatars.erase(id)

func creator_ambient(color: Array, brightness: float) -> void:
	var roots: Array = [world_root]
	roots.append_array(avatars.values())
	roots.append_array(entity_nodes.values())
	ambient_visuals.request(roots,Color(float(color[0]),float(color[1]),float(color[2])),brightness)

func _process(delta: float) -> void:
	ambient_visuals.advance()
	Materials.refresh_shadows()
	acoustics.listener(camera.global_position,get_world_3d())
	radios.environment(acoustics.route(camera.global_position))
	for state in voice.streams.values():
		state.player.bus = acoustics.route(state.player.global_position, int(state.get("route", 0)) == 1)
		acoustics.source_gain(state.player,int(state.get("route",0)) == 1)
	for surface in video_surfaces.values():
		surface.player.bus = acoustics.route(surface.global_position)
		surface.player.volume_linear = acoustics.source_gain(surface.player)
	for state in radios.receiver.streams.values():
		acoustics.source_gain(state.player)
	for device in radios.devices.values():
		device.tone.volume_linear = acoustics.source_gain(device.tone)
	for audio in get_tree().get_nodes_in_group("ssc_packaged_audio"):
		audio.environment(acoustics)
	if world_root and not avatar_loader and not avatar_queue.is_empty():
		_load_next_avatar()
	# Head and hands are tracked every render frame, independently of ENet TPS.
	var poses: Array[Transform3D] = [camera.global_transform]
	for i in 2:
		if xr_active and controllers[i].get_is_active():
			poses.append(controllers[i].global_transform)
		else:
			var hand := camera.global_transform
			hand.origin += camera.global_basis * Vector3(-0.3 if i == 0 else 0.3, -0.5, -0.3)
			poses.append(hand)
	var tracked: Dictionary = body_tracking.collect(origin.global_transform) if xr_active else {}
	network.pose(delta, poses, tracked)
	if avatars.has(network.user):
		avatars[network.user].local_pose(delta, poses, sleeping, tracked)
	var constraints: Dictionary = {}
	for contact in interactions.values():
		if not input_focused and (str(contact.actor) == network.user or str(contact.target) == network.user):
			continue
		if avatars.has(str(contact.actor)) and avatars.has(str(contact.target)):
			var list: Array = constraints.get(str(contact.target), [])
			list.append({"bone":str(contact.bone), "position":avatars[str(contact.actor)].hands[int(contact.hand)].global_position})
			constraints[str(contact.target)] = list
	for user in avatars:
		var list: Array[Dictionary] = []
		for constraint in constraints.get(user, []):
			list.append(constraint)
		avatars[user].grabs(list)
	if not input_focused:
		return
	touch_elapsed += delta
	if touch_elapsed > 0.5 and bool(preferences.get("can_touch_others", false)):
		touch_elapsed = 0
		_contact_nearest("touch", 0.3)
	if xr_active:
		phone.global_transform = controllers[phone_hand].global_transform
		phone.rotate_object_local(Vector3.RIGHT, -0.6)
		phone.translate_object_local(Vector3(0, 0.2, -0.08))
		var stick := controllers[1].get_vector2("primary")
		if not menu_pressed:
			var direction := camera.global_basis * Vector3(stick.x, 0, -stick.y)
			direction.y = 0
			origin.position += direction * delta * 1.5
	else:
		var direction := Vector3.ZERO
		if not grant_input.has_focus() and not chat_input.has_focus():
			direction = Vector3(float(Input.is_physical_key_pressed(KEY_D)) - float(Input.is_physical_key_pressed(KEY_A)), 0, float(Input.is_physical_key_pressed(KEY_S)) - float(Input.is_physical_key_pressed(KEY_W)))
		origin.position += origin.basis * direction * delta * 2
	if network.joined and not selected_entity.is_empty():
		drive_elapsed += delta
		if drive_elapsed >= 1.0 / network.send_hz and entity_data.has(selected_entity):
			drive_elapsed = 0
			var entity: Dictionary = entity_data[selected_entity]
			if entity.get("controller") == network.user and entity.get("component") == "vehicle":
				var throttle := float(Input.is_physical_key_pressed(KEY_UP)) - float(Input.is_physical_key_pressed(KEY_DOWN))
				var turn := float(Input.is_physical_key_pressed(KEY_RIGHT)) - float(Input.is_physical_key_pressed(KEY_LEFT))
				if xr_active:
					var stick := controllers[1 - phone_hand].get_vector2("primary")
					throttle = stick.y
					turn = stick.x
				network.input_action({"action":"drive", "target":selected_entity, "throttle":throttle, "turn":turn})
	if menu_pressed:
		menu_held += delta
		radial.visible = menu_held >= menu_hold_delay
		if radial.visible:
			phone.visible = true
			radial.point(controllers[1 - phone_hand].get_vector2("primary"))

func _controller_pressed(action: StringName) -> void:
	if not input_focused:
		return
	if action == &"menu_button" or action == &"ax_button":
		menu_pressed = true
		menu_held = 0
		menu_hold_delay = RadialMenu.hold_delay(float(preferences.get("radial_hold_seconds", 0.45)))
		radial.configure_actions(preferences.get("radial_actions", RadialMenu.DEFAULT_ACTIONS))
		phone_before_radial = phone.visible
	elif action == &"trigger_click":
		if not _point_phone(true):
			_fire()

func _controller_released(action: StringName) -> void:
	if not input_focused:
		return
	if action == &"menu_button" or action == &"ax_button":
		if not menu_pressed:
			return
		if menu_held >= menu_hold_delay:
			phone.visible = phone_before_radial
			match radial.selected_action():
				"microphone": voice.set_microphone(not voice.microphone_enabled)
				"sleep": _sleep()
				"phone_hand": _switch_phone_hand()
				"interact": _interact()
				"avatar_action": network.command(13, {"name":avatar_action_input.text, "weight":1.0})
				"video_pause": _media_command("pause")
				"phone": phone.visible = true
				"leave":
					network.close()
					_clear_instance()
		else:
			phone.visible = not phone.visible
		menu_pressed = false
		radial.visible = false
	elif action == &"trigger_click":
		_point_phone(false)

func _switch_phone_hand() -> void:
	phone_hand = 1 - phone_hand
	if not preferences.is_empty():
		preferences.phone_hand = "left" if phone_hand == 0 else "right"
		_save_preferences()

func _point_phone(pressed: bool) -> bool:
	if not phone.visible:
		return false
	var pointer := controllers[1 - phone_hand]
	var from := phone.to_local(pointer.global_position)
	var direction := phone.global_basis.inverse() * -pointer.global_basis.z
	if absf(direction.z) < 0.0001:
		return false
	var t := -from.z / direction.z
	if t < 0:
		return false
	var point := from + direction * t
	if absf(point.x) > 0.14 or absf(point.y) > 0.24:
		return false
	var event := InputEventMouseButton.new()
	event.position = Vector2((point.x / 0.28 + 0.5) * 560, (0.5 - point.y / 0.48) * 960)
	event.button_index = MOUSE_BUTTON_LEFT
	event.pressed = pressed
	phone_viewport.push_input(event, true)
	return true

func _interact() -> void:
	var nearest := ""
	var distance := 4.0
	for id in entity_nodes:
		if not entity_data.has(id) or not entity_data[id].get("component", "") in ["switch", "door"]:
			continue
		var d: float = camera.global_position.distance_to(entity_nodes[id].global_position)
		if d < distance:
			distance = d
			nearest = str(id)
	if not nearest.is_empty():
		network.command(5, {"target": nearest})

func _claim(component: String) -> void:
	var nearest := ""
	var distance := 4.0
	for id in entity_data:
		if entity_data[id].component == component:
			var d: float = camera.global_position.distance_to(entity_nodes[id].global_position)
			if d < distance:
				distance = d
				nearest = str(id)
	if not nearest.is_empty():
		selected_entity = nearest
		var command := {"action":"grab" if component == "prop" else "lease", "target":nearest}
		if component == "prop":
			command.hand = 1 - phone_hand
		network.command(12, command)

func _fire() -> void:
	if not selected_entity.is_empty():
		network.command(12, {"action":"fire", "target":selected_entity})

func _release() -> void:
	if not selected_entity.is_empty():
		network.command(12, {"action":"release", "target":selected_entity})
		selected_entity = ""

func _unhandled_input(event: InputEvent) -> void:
	if not input_focused:
		return
	if xr_active:
		return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT:
		Input.mouse_mode = Input.MOUSE_MODE_CAPTURED if event.pressed else Input.MOUSE_MODE_VISIBLE
	elif event is InputEventMouseMotion and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
		origin.rotate_y(-event.relative.x * 0.003)
		camera.rotation.x = clampf(camera.rotation.x - event.relative.y * 0.003, -1.4, 1.4)
	elif event is InputEventKey and event.pressed and not event.echo and event.physical_keycode == KEY_SPACE and not chat_input.has_focus() and not grant_input.has_focus():
		_fire()

func _sleep() -> void:
	sleeping = not sleeping
	network.command(7, {"sleeping": sleeping})
	if not preferences.is_empty():
		preferences.sleeping = sleeping
		_save_preferences()

func _status(text: String) -> void:
	if status_label:
		status_label.text = text
	print(text)

func _load_next_avatar() -> void:
	var item: Dictionary = avatar_queue.pop_front()
	if not avatars.has(item.player) or avatars[item.player].avatar_version != item.version:
		return
	avatar_loader = Loader.new()
	avatar_loader.credentials = loader.credentials
	avatar_loader.base_url = loader.base_url
	avatar_loader.quality = quality
	avatar_loader.cache_notice.connect(_status)
	avatar_loader.texture_reduction = int(preferences.get("texture_reduction", 0))
	avatar_loader.local_avatar = item.player == network.user
	add_child(avatar_loader)
	var active_loader: Node = avatar_loader
	active_loader.loaded.connect(func(root: Node3D):
		if avatars.has(item.player) and avatars[item.player].avatar_version == item.version:
			avatars[item.player].set_model(root, active_loader.staging, active_loader.manifest)
		else:
			root.free()
		active_loader.queue_free()
		if avatar_loader == active_loader:
			avatar_loader = null
	)
	active_loader.failed.connect(func(text: String):
		_status("Avatar fallback: " + text)
		active_loader.queue_free()
		if avatar_loader == active_loader:
			avatar_loader = null
	)
	active_loader.load_avatar(str(item.asset), str(item.version))

func _avatar_catalog(result: int, code: int, _headers: PackedStringArray, body: PackedByteArray) -> void:
	if result != HTTPRequest.RESULT_SUCCESS or code != 200:
		_status("Could not fetch your avatar slots")
		return
	var assets = JSON.parse_string(body.get_string_from_utf8())
	if not assets is Array:
		return
	avatar_choices.clear()
	avatar_choices.add_item("Fallback avatar")
	avatar_choices.set_item_metadata(0, null)
	for asset in assets:
		avatar_choices.add_item(str(asset.name))
		avatar_choices.set_item_metadata(avatar_choices.item_count - 1, str(asset.id))

func _preferences_received(result: int, code: int, _headers: PackedStringArray, body: PackedByteArray) -> void:
	if result != HTTPRequest.RESULT_SUCCESS or code != 200:
		_status("Could not load/save preferences")
		return
	var data = JSON.parse_string(body.get_string_from_utf8())
	if data is Dictionary and data.has("preferences"):
		preferences = data.preferences
		_apply_preferences()
		network.command(7, {"sleeping":sleeping})
	else:
		_status("Preferences saved")

func _apply_preferences() -> void:
	phone_hand = 0 if preferences.phone_hand == "left" else 1
	quality = int(preferences.quality)
	sleeping = bool(preferences.sleeping)
	radio_preset = str(preferences.get("radio_preset", "standard"))
	_apply_radio_preset()
	loader.quality = quality
	loader.texture_reduction = int(preferences.get("texture_reduction", 0))
	if world_root:
		Materials.apply(world_root, quality)
	for avatar in avatars.values():
		Materials.apply(avatar, quality)
	for entity in entity_nodes.values():
		Materials.apply(entity, quality)
	settings_panel.configure(preferences)
	var notices := SettingsPanel.apply_local(preferences, get_viewport(), voice, radios, camera)
	settings_panel.report(notices)
	if not notices.is_empty():
		_status("\n".join(notices))

func _save_preferences() -> void:
	preference_save_pending = not preferences.is_empty()
	preference_save_timer.start()

func _flush_preferences() -> void:
	if not preference_save_pending:
		return
	var payload := preferences.duplicate(true)
	for key in ["quality", "send_divisor", "receive_divisor", "texture_reduction"]:
		payload[key] = int(payload[key])
	var error := preferences_request.request(loader.base_url + "/api/v1/preferences", PackedStringArray(["Authorization: Bearer " + loader.credentials,"Content-Type: application/json"]), HTTPClient.METHOD_PUT, JSON.stringify(payload))
	if error == OK:
		preference_save_pending = false
	else:
		preference_save_timer.start() # One coalesced pending state, never a request queue.

func _contact_nearest(action: String, reach: float = 0.45) -> void:
	if not network.joined or not avatars.has(network.user):
		return
	var hand := 1 - phone_hand
	var position: Vector3 = avatars[network.user].hands[hand].global_position
	for user in avatars:
		if user == network.user:
			continue
		var candidates: Array = avatars[user].colliders if action == "grab" else [{"id":"head","anchor":"head","offset":[0,0,0],"radius":0.18}]
		for collider in candidates:
			if action == "grab" and not bool(collider.get("grabbable", false)):
				continue
			var anchor: Transform3D = avatars[user].head.global_transform
			if collider.anchor in ["left_hand", "right_hand"]:
				anchor = avatars[user].hands[0 if collider.anchor == "left_hand" else 1].global_transform
			var offset: Array = collider.offset
			var center := anchor * Vector3(float(offset[0]),float(offset[1]),float(offset[2]))
			if position.distance_to(center) < minf(reach, float(collider.radius) + 0.12):
				network.command(6, {"action":action,"target":user,"hand":hand,"segment":collider.id})
				return

func _exit_tree() -> void:
	network.close()
	vrm_runtime.close()
