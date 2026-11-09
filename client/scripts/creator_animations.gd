extends RefCounted
## Uploaded clips may animate only supplied transforms/bones/blend shapes.
## Method/audio/property tracks cannot call host code or mutate arbitrary state.
const Visuals = preload("res://scripts/creator_visuals.gd")

static func normalize(root: Node) -> void:
	var players: Array[Node] = root.find_children("*","AnimationPlayer",true,false)
	if root is AnimationPlayer:
		players.append(root)
	for player: AnimationPlayer in players:
		player.autoplay = ""
		player.stop()
		var base: Node = player.get_node_or_null(player.root_node) if not player.root_node.is_absolute() else null
		for library_name in player.get_animation_library_list():
			var supplied := player.get_animation_library(library_name)
			var library := AnimationLibrary.new()
			for name in supplied.get_animation_list():
				var animation := supplied.get_animation(name).duplicate() as Animation
				for track in range(animation.get_track_count() - 1,-1,-1):
					var kind := animation.track_get_type(track)
					var path := animation.track_get_path(track)
					var target: Node = base.get_node_or_null(NodePath(path.get_concatenated_names())) if base and not path.is_absolute() else null
					var allowed := target != null and (target == root or root.is_ancestor_of(target))
					if kind == Animation.TYPE_BLEND_SHAPE:
						allowed = allowed and target is MeshInstance3D and path.get_subname_count() == 1 and target.find_blend_shape_by_name(path.get_subname(0)) >= 0
					elif kind in [Animation.TYPE_POSITION_3D,Animation.TYPE_ROTATION_3D,Animation.TYPE_SCALE_3D]:
						allowed = allowed and target is Node3D and (path.get_subname_count() == 0 or (target is Skeleton3D and path.get_subname_count() == 1 and target.find_bone(path.get_subname(0)) >= 0))
					else:
						allowed = false
					if not allowed:
						animation.remove_track(track)
				library.add_animation(name,animation)
			player.remove_animation_library(library_name)
			player.add_animation_library(library_name,library)

static func apply(scope: Node, attachment: String, command: Dictionary) -> String:
	var target := Visuals.target(scope,str(command.target),attachment)
	if not target:
		return "Unknown, ambiguous or out-of-scope animation target"
	var players := Visuals.descendants(target,"AnimationPlayer")
	if players.size() != 1:
		return "Animation target must contain exactly one supplied AnimationPlayer"
	var player := players[0] as AnimationPlayer
	match str(command.action):
		"play_animation":
			if not player.has_animation(str(command.clip)):
				return "Unknown supplied animation clip"
			player.play(str(command.clip),0,float(command.value))
		"stop_animation": player.stop()
		"seek_animation":
			if player.assigned_animation.is_empty() or not player.has_animation(player.assigned_animation):
				return "No assigned animation to seek"
			if float(command.value) > player.get_animation(player.assigned_animation).length:
				return "Animation seek exceeds clip length"
			player.seek(float(command.value),true)
	return ""
