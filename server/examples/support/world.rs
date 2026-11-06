//! Small, deterministic authoring fixture using the backend's existing libraries.
use serde_json::json;
use std::io::{Seek, Write};

pub fn write(destination: impl Write + Seek) -> anyhow::Result<()> {
    write_kind(destination, false)
}
pub fn write_tanks(destination: impl Write + Seek) -> anyhow::Result<()> {
    write_kind(destination, true)
}
fn write_kind(destination: impl Write + Seek, tanks: bool) -> anyhow::Result<()> {
    let vertices: [f32; 12] = [-5., 0., -5., 5., 0., -5., 5., 0., 5., -5., 0., 5.];
    let mut binary = Vec::new();
    for value in vertices {
        binary.extend(value.to_le_bytes());
    }
    for index in [0u16, 2, 1, 0, 3, 2] {
        binary.extend(index.to_le_bytes());
    }
    let doc = json!({
        "asset": {"version":"2.0", "generator":"Super Simple Chat in VR"},
        "scene":0, "scenes":[{"nodes":[0]}], "nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0}]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.15,0.28,0.34,1],"metallicFactor":0,"roughnessFactor":1},"doubleSided":true}],
        "buffers":[{"byteLength":binary.len()}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":48},{"buffer":0,"byteOffset":48,"byteLength":12}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[-5,0,-5],"max":[5,0,5]}, {"bufferView":1,"componentType":5123,"count":6,"type":"SCALAR"}]
    });
    let mut encoded = serde_json::to_vec(&doc)?;
    while !encoded.len().is_multiple_of(4) {
        encoded.push(b' ');
    }
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend((28 + encoded.len() as u32 + binary.len() as u32).to_le_bytes());
    glb.extend((encoded.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(encoded);
    glb.extend((binary.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(binary);
    let mut manifest = json!({
        "schema":1,"kind":"world","name":"Quiet room",
        "description":"Reference world: authoritative switch, door, and sandboxed shared variable.",
        "scene":"world.glb","spawn":[0,0,0],"capacity":16,
        "variables":{"lights_enabled":{"type":"boolean","default":false}},
        "video_surfaces":[{"id":"screen","position":[0,2,-3],"size":[3.2,1.8],"public_controls":false}],
        "audio_zones":[{"id":"quiet_room","position":[0,1,0],"size":[6,3,6],"effects":{"reverb":0.2,"echo":0,"distortion":0,"compression":true}}],
        "radios":[{"id":"desk_radio","position":[0,1.2,-1],"channel":1}],
        "objects":[{"id":"switch","component":"switch","position":[0,1.2,-1],"size":[0.2,0.2,0.2]}, {"id":"door","component":"door","position":[1,1,-1],"size":[1,2,0.2]}, {"id":"radio_send","component":"switch","position":[0.5,1.2,-1],"size":[0.1,0.1,0.1]}],
        "scripts":[{"path":"scripts/main.luau","side":"server","attachment":"world","capabilities":["state"]}]
    });
    let mut script = "world.state.set(\"lights_enabled\", false)\nworld.on_interact(\"switch\", function(player)\n    local on = not world.state.get(\"lights_enabled\")\n    world.state.set(\"lights_enabled\", on)\n    world.set_active(\"door\", on)\nend)\n".to_string();
    script.push_str("world.on_interact('radio_send', function(player) world.radio_message('desk_radio', 'CQ CREATOR') end)\n");
    manifest["scripts"][0]["capabilities"] = json!(["state", "objects", "radio"]);
    if tanks {
        manifest["name"] = json!("Tank yard");
        manifest["description"] = json!(
            "Server-simulated tanks, packaged projectiles, hit detection, damage, score and control leases."
        );
        manifest["prefabs"] = json!({
            "tank":{"scene":"prefabs/tank.glb","component":"vehicle","physics":{"size":[1.5,0.8,2.0],"mass":100,"health":100,"speed":4,"projectile":"shell","fire_interval":0.2}},
            "shell":{"scene":"prefabs/shell.glb","component":"projectile","physics":{"shape":"sphere","size":[0.15,0.15,0.15],"mass":0.1,"gravity_scale":0,"damage":25,"speed":25,"lifetime":3}}
        });
        let objects = manifest["objects"].as_array_mut().unwrap();
        objects.extend([
            json!({"id":"ground","component":"static","position":[0,-0.1,0],"size":[20,0.2,20]}),
            json!({"id":"tank_a","component":"vehicle","prefab":"tank","position":[0,0.6,-1.5]}),
            json!({"id":"tank_b","component":"vehicle","prefab":"tank","position":[0,0.6,-7]}),
            json!({"id":"spawn_tank","component":"switch","position":[-1,1,-1]}),
            json!({"id":"crate","component":"prop","position":[-1,1,0],"physics":{"size":[0.3,0.3,0.3]}}),
        ]);
        script.push_str("world.on_interact(\"spawn_tank\", function(player)\n    world.spawn(\"tank\", {position={3, 1, -3}, owner=player.id})\nend)\n");
    }
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::from_date_and_time(2026, 10, 30, 0, 0, 0).unwrap())
        .unix_permissions(0o644);
    let mut archive = zip::ZipWriter::new(destination);
    for (name, content) in [
        ("manifest.json", serde_json::to_vec(&manifest)?),
        ("world.glb", glb),
        ("scripts/main.luau", script.into_bytes()),
    ] {
        archive.start_file(name, options)?;
        archive.write_all(&content)?;
    }
    if tanks {
        archive.start_file("prefabs/tank.glb", options)?;
        archive.write_all(&box_glb(true))?;
        archive.start_file("prefabs/shell.glb", options)?;
        archive.write_all(&box_glb(false))?;
    }
    archive.finish()?;
    Ok(())
}
fn box_glb(tank: bool) -> Vec<u8> {
    let mut binary = Vec::new();
    for p in [
        [-0.5f32, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ] {
        for v in p {
            binary.extend(v.to_le_bytes());
        }
    }
    for i in [
        0u16, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
        3, 1, 2, 6, 1, 6, 5,
    ] {
        binary.extend(i.to_le_bytes());
    }
    let (nodes, roots) = if tank {
        (
            json!([
                {"mesh":0,"scale":[1.5,0.6,2]},
                {"mesh":0,"scale":[0.8,0.3,0.8],"translation":[0,0.4,0]},
                {"mesh":0,"scale":[0.12,0.12,1.4],"translation":[0,0.4,-0.8]}
            ]),
            json!([0, 1, 2]),
        )
    } else {
        (json!([{"mesh":0,"scale":[0.15,0.15,0.15]}]), json!([0]))
    };
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":roots}],"nodes":nodes,
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0}]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":if tank {[0.25,0.5,0.3,1.0]}else{[1.0,0.6,0.1,1.0]},"metallicFactor":0,"roughnessFactor":1},"doubleSided":true}],
        "buffers":[{"byteLength":binary.len()}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":96},{"buffer":0,"byteOffset":96,"byteLength":72}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":8,"type":"VEC3","min":[-0.5,-0.5,-0.5],"max":[0.5,0.5,0.5]}, {"bufferView":1,"componentType":5123,"count":36,"type":"SCALAR"}]});
    let mut encoded = serde_json::to_vec(&doc).unwrap();
    while !encoded.len().is_multiple_of(4) {
        encoded.push(b' ');
    }
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend((28 + encoded.len() as u32 + binary.len() as u32).to_le_bytes());
    glb.extend((encoded.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(encoded);
    glb.extend((binary.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(binary);
    glb
}

/// Original procedural VRM fixture: a skinned cube, humanoid mapping, expression,
/// and a two-joint spring. No third-party avatar/model licenses are involved.
pub fn write_avatar(destination: impl Write + Seek) -> anyhow::Result<()> {
    let glb = box_glb(false);
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len])?;
    let mut binary = glb[28 + json_len..].to_vec();
    let joints_offset = binary.len();
    binary.extend([0u8; 32]);
    let weights_offset = binary.len();
    for _ in 0..8 {
        for value in [1.0f32, 0.0, 0.0, 0.0] {
            binary.extend(value.to_le_bytes());
        }
    }
    let morph_offset = binary.len();
    for i in 0..8 {
        for value in [if i % 2 == 0 { -0.05f32 } else { 0.05 }, 0.0, 0.0] {
            binary.extend(value.to_le_bytes());
        }
    }
    let views = doc["bufferViews"].as_array_mut().unwrap();
    for (offset, length) in [
        (joints_offset, 32),
        (weights_offset, 128),
        (morph_offset, 96),
    ] {
        views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":length}));
    }
    doc["accessors"].as_array_mut().unwrap().extend([
        json!({"bufferView":2,"componentType":5121,"count":8,"type":"VEC4"}),
        json!({"bufferView":3,"componentType":5126,"count":8,"type":"VEC4"}),
        json!({"bufferView":4,"componentType":5126,"count":8,"type":"VEC3","min":[-0.05,0,0],"max":[0.05,0,0]})]);
    doc["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(2);
    doc["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(3);
    doc["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION":4}]);
    doc["meshes"][0]["weights"] = json!([0]);
    doc["meshes"][0]["extras"] = json!({"targetNames":["Happy"]});
    let bones = [
        ("hips", 0, [0.0f32, 1.0, 0.0]),
        ("spine", 1, [0.0, 0.2, 0.0]),
        ("chest", 2, [0.0, 0.2, 0.0]),
        ("head", 3, [0.0, 0.3, 0.0]),
        ("leftUpperLeg", 1, [0.12, 0.0, 0.0]),
        ("leftLowerLeg", 5, [0.0, -0.4, 0.0]),
        ("leftFoot", 6, [0.0, -0.4, -0.05]),
        ("rightUpperLeg", 1, [-0.12, 0.0, 0.0]),
        ("rightLowerLeg", 8, [0.0, -0.4, 0.0]),
        ("rightFoot", 9, [0.0, -0.4, -0.05]),
        ("leftUpperArm", 3, [0.25, 0.1, 0.0]),
        ("leftLowerArm", 11, [0.3, 0.0, 0.0]),
        ("leftHand", 12, [0.25, 0.0, 0.0]),
        ("rightUpperArm", 3, [-0.25, 0.1, 0.0]),
        ("rightLowerArm", 14, [-0.3, 0.0, 0.0]),
        ("rightHand", 15, [-0.25, 0.0, 0.0]),
        ("HairRoot", 4, [0.0, 0.12, 0.0]),
        ("HairTip", 17, [0.0, 0.2, 0.0]),
    ];
    let mut nodes = vec![json!({"name":"Body","mesh":0,"skin":0})];
    let mut mapping = serde_json::Map::new();
    for (index, (name, _, position)) in bones.iter().enumerate() {
        nodes.push(json!({"name":name,"translation":position,"children":[]}));
        if index < 16 {
            mapping.insert(name.to_string(), json!({"node":index+1}));
        }
    }
    for (index, (_, parent, _)) in bones.iter().enumerate() {
        if *parent > 0 {
            nodes[*parent]["children"]
                .as_array_mut()
                .unwrap()
                .push(json!(index + 1));
        }
    }
    let body = nodes[0].clone();
    nodes[0] = json!({"name":"Avatar","children":[19,1]});
    nodes.push(body);
    doc["nodes"] = json!(nodes);
    doc["scenes"] = json!([{"nodes":[0]}]);
    doc["skins"] = json!([{"joints":(1..=18).collect::<Vec<_>>(),"skeleton":1}]);
    doc["buffers"][0]["byteLength"] = json!(binary.len());
    doc["extensionsUsed"] = json!(["VRMC_vrm", "VRMC_springBone"]);
    doc["extensions"] = json!({
        "VRMC_vrm":{"specVersion":"1.0","meta":{"name":"Reference avatar","authors":["Super Simple Chat in VR"],"version":"1","licenseUrl":"https://vrm.dev/licenses/1.0/"},
            "humanoid":{"humanBones":mapping},"expressions":{"preset":{"happy":{"morphTargetBinds":[{"node":19,"index":0,"weight":1}]}}}},
        "VRMC_springBone":{"specVersion":"1.0","colliders":[],"colliderGroups":[],"springs":[{"name":"Hair","joints":[{"node":17,"hitRadius":0.02,"stiffness":1,"gravityPower":1,"gravityDir":[0,-1,0],"dragForce":0.4},{"node":18}]}]}
    });
    let mut encoded = serde_json::to_vec(&doc)?;
    while !encoded.len().is_multiple_of(4) {
        encoded.push(b' ');
    }
    let mut avatar = b"glTF".to_vec();
    avatar.extend(2u32.to_le_bytes());
    avatar.extend((28 + encoded.len() as u32 + binary.len() as u32).to_le_bytes());
    avatar.extend((encoded.len() as u32).to_le_bytes());
    avatar.extend(b"JSON");
    avatar.extend(encoded);
    avatar.extend((binary.len() as u32).to_le_bytes());
    avatar.extend(b"BIN\0");
    avatar.extend(binary);
    let manifest = json!({"schema":1,"kind":"avatar","name":"Reference avatar","scene":"avatar.vrm","actions":["happy"],"contacts":[{"id":"hair","bone":"HairRoot","anchor":"head","offset":[0,0.12,0],"radius":0.2,"grabbable":true}],"scripts":[{"path":"scripts/visual.luau","side":"client","attachment":"avatar","capabilities":["visual"]}]});
    let mut archive = zip::ZipWriter::new(destination);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (name, bytes) in [
        ("manifest.json", serde_json::to_vec(&manifest)?),
        ("avatar.vrm", avatar),
        (
            "scripts/visual.luau",
            b"avatar.on('action', function(e) avatar.expression(e.name, e.weight) end)".to_vec(),
        ),
    ] {
        archive.start_file(name, options)?;
        archive.write_all(&bytes)?;
    }
    archive.finish()?;
    Ok(())
}
