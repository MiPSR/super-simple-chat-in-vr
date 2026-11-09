extends RefCounted
const Main = preload("res://scripts/main.gd")
const Network = preload("res://scripts/network.gd")
const Voice = preload("res://scripts/voice.gd")

class Transport extends Network:
	var commands: Array[Dictionary] = []
	func command(kind: int, data: Dictionary) -> void:
		commands.append({"kind":kind,"data":data})

static func run() -> void:
	var tree: SceneTree = Engine.get_main_loop()
	var network := Transport.new()
	tree.root.add_child(network)
	network.set_process(false)
	network.joined = true
	var voice := Voice.new()
	tree.root.add_child(voice)
	voice.set_process(false)
	voice.attach(network)
	voice.codec = RefCounted.new()
	voice.microphone_enabled = true # No physical capture is started by this fixture.
	voice.radio_target = "radio"
	var main := Main.new()
	main.network = network
	main.voice = voice
	main.phone = Node3D.new()
	main.radial = Control.new()
	main.add_child(main.phone)
	main.add_child(main.radial)
	main.phone.visible = true
	main.radial.visible = true
	main.selected_entity = "prop"
	main.menu_pressed = true
	main._set_input_focus(false)
	assert(not voice.microphone_enabled and not voice.capture_permitted and voice.radio_target.is_empty())
	assert(not network.input_enabled and int(network.commands.back().kind) == 28 and not network.commands.back().data.active)
	assert(not main.menu_pressed and not main.radial.visible and not main.phone.visible and main.selected_entity.is_empty())
	voice.set_microphone(true)
	assert(not voice.microphone_enabled)
	network.pose(1,[])
	network.input_action({"action":"fire"})
	network.voice(0,0,PackedByteArray([1]))
	main._set_input_focus(true)
	assert(voice.capture_permitted and not voice.microphone_enabled)
	assert(network.input_enabled and network.commands.back().data.active)
	assert(main.phone.visible and not main.menu_pressed)
	main.free()
	voice.free()
	network.free()
	print("PASS focus-loss capture/input suppression, reliable focus state, cancelled gestures and explicit microphone resume")
