//! Bounded glTF scene references and importer-facing shapes, beyond byte ranges.
use super::{ApiError,ApiResult};
use serde_json::Value;

fn array(value: &Value, maximum: usize) -> ApiResult<&[Value]> {
    match value {
        Value::Null => Ok(&[]),
        Value::Array(values) if values.len() <= maximum => Ok(values),
        _ => Err(ApiError::bad("Invalid/oversized glTF scene array")),
    }
}
fn index(value: &Value, length: usize) -> ApiResult<usize> {
    value.as_u64().filter(|i|*i < length as u64).map(|i|i as usize)
        .ok_or_else(||ApiError::bad("glTF scene reference out of bounds"))
}
fn vector(value: &Value, length: usize, bound: f64) -> bool {
    value.as_array().is_some_and(|values|values.len()==length && values.iter()
        .all(|value|value.as_f64().is_some_and(|v|v.is_finite() && v.abs() <= bound)))
}
fn shape(accessor: &Value, kind: &str, components: &[u64]) -> bool {
    accessor["type"].as_str()==Some(kind)
        && accessor["componentType"].as_u64().is_some_and(|c|components.contains(&c))
}
fn morph_count(mesh: &Value) -> usize {
    mesh["primitives"].as_array().and_then(|p|p.first())
        .and_then(|p|p["targets"].as_array()).map_or(0,Vec::len)
}

pub fn validate(doc: &Value) -> ApiResult<()> {
    let accessors=array(&doc["accessors"],16384)?;
    let meshes=array(&doc["meshes"],4096)?;
    let nodes=array(&doc["nodes"],4096)?;
    let skins=array(&doc["skins"],256)?;
    let materials=array(&doc["materials"],1024)?;
    let textures=array(&doc["textures"],256)?;
    let images=array(&doc["images"],256)?;
    let samplers=array(&doc["samplers"],256)?;
    let cameras=array(&doc["cameras"],256)?;
    let lights=array(&doc["extensions"]["KHR_lights_punctual"]["lights"],64)?;
    for light in lights {
        if !matches!(light["type"].as_str(),Some("point"|"spot"|"directional"))
            || light.get("color").is_some_and(|value|!value.as_array().is_some_and(|values|values.len()==3 && values.iter().all(|v|scalar(v,0.0,1.0))))
            || light.get("intensity").is_some_and(|value|!scalar(value,0.0,1_000_000.0))
            || light.get("range").is_some_and(|value|!scalar(value,0.0001,100000.0)) {
            return Err(ApiError::bad("Invalid glTF punctual light"));
        }
        for name in ["innerConeAngle","outerConeAngle"] {
            if light["spot"].get(name).is_some_and(|value|!scalar(value,0.0,std::f64::consts::FRAC_PI_2)) {
                return Err(ApiError::bad("Invalid glTF spotlight angle"));
            }
        }
        let inner=light["spot"]["innerConeAngle"].as_f64().unwrap_or(0.0);
        let outer=light["spot"]["outerConeAngle"].as_f64().unwrap_or(std::f64::consts::FRAC_PI_4);
        if inner>outer || outer<=0.0 { return Err(ApiError::bad("Invalid spotlight cone interval")); }
    }
    for accessor in accessors {
        let components=match accessor["type"].as_str() {
            Some("SCALAR")=>1,Some("VEC2")=>2,Some("VEC3")=>3,Some("VEC4"|"MAT2")=>4,
            Some("MAT3")=>9,Some("MAT4")=>16,_=>return Err(ApiError::bad("Invalid accessor shape")),
        };
        if accessor.get("normalized").is_some_and(|value|!value.is_boolean() || (value.as_bool()==Some(true) && accessor["componentType"]==5126)) {
            return Err(ApiError::bad("Invalid accessor normalization"));
        }
        for name in ["min","max"] {
            if accessor.get(name).is_some_and(|value|!vector(value,components,100_000_000.0)) {
                return Err(ApiError::bad("Invalid accessor bounds"));
            }
        }
        if let (Some(minimum),Some(maximum))=(accessor["min"].as_array(),accessor["max"].as_array())
            && minimum.iter().zip(maximum).any(|(a,b)|a.as_f64().unwrap()>b.as_f64().unwrap()) {
            return Err(ApiError::bad("Reversed accessor bounds"));
        }
    }
    let mut geometry=0u64;
    let mut primitives=0usize;
    for mesh in meshes {
        let entries=array(&mesh["primitives"],4096)?;
        if entries.is_empty() { return Err(ApiError::bad("glTF mesh has no primitives")); }
        let morphs=morph_count(mesh);
        if mesh.get("weights").is_some_and(|weights|!vector(weights,morphs,10000.0)) {
            return Err(ApiError::bad("Invalid glTF mesh morph weights"));
        }
        for primitive in entries {
            primitives+=1;
            if primitives > 4096 { return Err(ApiError::bad("glTF primitive budget exceeded")); }
            let attributes=primitive["attributes"].as_object()
                .filter(|attributes|attributes.len()<=16)
                .ok_or_else(||ApiError::bad("Invalid glTF primitive attributes"))?;
            let position=&accessors[index(&primitive["attributes"]["POSITION"],accessors.len())?];
            if !shape(position,"VEC3",&[5126]) {
                return Err(ApiError::bad("glTF positions must be float VEC3"));
            }
            let count=position["count"].as_u64().unwrap_or(0);
            for (name,reference) in attributes {
                let accessor=&accessors[index(reference,accessors.len())?];
                if accessor["count"].as_u64()!=Some(count) {
                    return Err(ApiError::bad("glTF attribute counts disagree"));
                }
                let valid=match name.as_str() {
                    "POSITION" | "NORMAL"=>shape(accessor,"VEC3",&[5126]),
                    "TANGENT"=>shape(accessor,"VEC4",&[5126]),
                    "TEXCOORD_0" | "TEXCOORD_1" | "TEXCOORD_2" | "TEXCOORD_3"=>shape(accessor,"VEC2",&[5121,5123,5126]),
                    "COLOR_0"=>shape(accessor,"VEC3",&[5121,5123,5126]) || shape(accessor,"VEC4",&[5121,5123,5126]),
                    "JOINTS_0" | "JOINTS_1"=>shape(accessor,"VEC4",&[5121,5123]) && accessor.get("normalized").is_none_or(|v|v.as_bool()==Some(false)),
                    "WEIGHTS_0" | "WEIGHTS_1"=>shape(accessor,"VEC4",&[5121,5123,5126]),
                    _=>false,
                };
                if !valid { return Err(ApiError::bad("Unsupported/incompatible glTF vertex attribute")); }
            }
            for suffix in ["0","1"] {
                if attributes.contains_key(&format!("JOINTS_{suffix}")) != attributes.contains_key(&format!("WEIGHTS_{suffix}")) {
                    return Err(ApiError::bad("glTF joints/weights must be paired"));
                }
            }
            let elements=if let Some(reference)=primitive.get("indices") {
                let accessor=&accessors[index(reference,accessors.len())?];
                if !shape(accessor,"SCALAR",&[5121,5123,5125]) || accessor.get("normalized").is_some_and(|v|v.as_bool()!=Some(false)) {
                    return Err(ApiError::bad("Invalid glTF index accessor"));
                }
                accessor["count"].as_u64().unwrap_or(0)
            } else { count };
            geometry=geometry.checked_add(elements.max(count)).filter(|n|*n<=4_000_000)
                .ok_or_else(||ApiError::bad("glTF geometry element budget exceeded"))?;
            if primitive.get("mode").is_some_and(|v|v.as_u64().is_none_or(|mode|mode>6)) {
                return Err(ApiError::bad("Invalid glTF primitive mode"));
            }
            if let Some(reference)=primitive.get("material") { index(reference,materials.len())?; }
            let targets=array(&primitive["targets"],64)?;
            if targets.len()!=morphs { return Err(ApiError::bad("glTF primitive morph target counts disagree")); }
            for target in targets {
                let target=target.as_object().filter(|target|target.len()<=3)
                    .ok_or_else(||ApiError::bad("Invalid morph target"))?;
                for (name,reference) in target {
                    let accessor=&accessors[index(reference,accessors.len())?];
                    if !matches!(name.as_str(),"POSITION"|"NORMAL"|"TANGENT")
                        || !shape(accessor,"VEC3",&[5126]) || accessor["count"].as_u64()!=Some(count) {
                        return Err(ApiError::bad("Invalid morph target accessor"));
                    }
                }
            }
        }
    }
    for node in nodes {
        if let Some(reference)=node.pointer("/extensions/KHR_lights_punctual/light") { index(reference,lights.len())?; }
        for (property,length) in [("mesh",meshes.len()),("skin",skins.len()),("camera",cameras.len())] {
            if let Some(reference)=node.get(property) { index(reference,length)?; }
        }
        if node.get("skin").is_some() && node.get("mesh").is_none() {
            return Err(ApiError::bad("Skinned glTF node requires a mesh"));
        }
        if let Some(weights)=node.get("weights") {
            let mesh=index(&node["mesh"],meshes.len())?;
            if !vector(weights,morph_count(&meshes[mesh]),10000.0) {
                return Err(ApiError::bad("Invalid glTF node morph weights"));
            }
        }
        for (property,length,bound) in [("translation",3,100000.0),("scale",3,10000.0),("rotation",4,1.0),("matrix",16,100000.0)] {
            if node.get(property).is_some_and(|value|!vector(value,length,bound)) {
                return Err(ApiError::bad("Invalid/nonfinite glTF node transform"));
            }
        }
        if node.get("matrix").is_some() && ["translation","rotation","scale"].iter().any(|key|node.get(key).is_some()) {
            return Err(ApiError::bad("glTF node cannot combine matrix and TRS"));
        }
        if let Some(rotation)=node["rotation"].as_array()
            && (rotation.iter().map(|v|v.as_f64().unwrap().powi(2)).sum::<f64>()-1.0).abs()>0.01 {
            return Err(ApiError::bad("Invalid glTF node quaternion"));
        }
    }
    for skin in skins {
        let joints=array(&skin["joints"],256)?;
        if joints.is_empty() { return Err(ApiError::bad("glTF skin has no joints")); }
        let mut unique=std::collections::HashSet::new();
        for joint in joints {
            if !unique.insert(index(joint,nodes.len())?) { return Err(ApiError::bad("Duplicate glTF skin joint")); }
        }
        if let Some(reference)=skin.get("skeleton") { index(reference,nodes.len())?; }
        if let Some(reference)=skin.get("inverseBindMatrices") {
            let accessor=&accessors[index(reference,accessors.len())?];
            if !shape(accessor,"MAT4",&[5126]) || accessor["count"].as_u64()!=Some(joints.len() as u64) {
                return Err(ApiError::bad("Invalid glTF inverse bind matrices"));
            }
        }
    }
    for texture in textures {
        let sources=[texture.get("source"),texture.pointer("/extensions/EXT_texture_webp/source")];
        if sources.iter().all(Option::is_none) { return Err(ApiError::bad("glTF texture requires embedded image")); }
        for source in sources.into_iter().flatten() { index(source,images.len())?; }
        if let Some(reference)=texture.get("sampler") { index(reference,samplers.len())?; }
    }
    for sampler in samplers {
        for (name,allowed) in [("magFilter",&[9728u64,9729][..]),("minFilter",&[9728,9729,9984,9985,9986,9987][..]),
            ("wrapS",&[33071,33648,10497][..]),("wrapT",&[33071,33648,10497][..])] {
            if sampler.get(name).is_some_and(|value|value.as_u64().is_none_or(|value|!allowed.contains(&value))) {
                return Err(ApiError::bad("Invalid glTF texture sampler"));
            }
        }
    }
    for material in materials {
        for path in ["/pbrMetallicRoughness/baseColorTexture/index","/pbrMetallicRoughness/metallicRoughnessTexture/index",
            "/normalTexture/index","/occlusionTexture/index","/emissiveTexture/index"] {
            if let Some(reference)=material.pointer(path) { index(reference,textures.len())?; }
        }
    }
    animations(doc,accessors,nodes)?;
    vrm(doc,nodes,meshes,materials.len())?;
    Ok(())
}

fn scalar(value: &Value, minimum: f64, maximum: f64) -> bool {
    value.as_f64().is_some_and(|value|value.is_finite() && (minimum..=maximum).contains(&value))
}

fn vrm(doc: &Value, nodes: &[Value], meshes: &[Value], materials: usize) -> ApiResult<()> {
    if let Some(vrm)=doc.pointer("/extensions/VRMC_vrm") {
        if let Some(bones)=vrm.pointer("/humanoid/humanBones") {
            let bones=bones.as_object().filter(|bones|bones.len()<=128)
                .ok_or_else(||ApiError::bad("Invalid VRM humanoid bone map"))?;
            let mut unique=std::collections::HashSet::new();
            for (name,bone) in bones {
                if name.is_empty() || name.len()>64 || !unique.insert(index(&bone["node"],nodes.len())?) {
                    return Err(ApiError::bad("Invalid/duplicate VRM humanoid bone"));
                }
            }
        }
        let mut bind_count=0usize;
        for category in ["preset","custom"] {
            if let Some(expressions)=vrm.pointer(&format!("/expressions/{category}")) {
                let expressions=expressions.as_object().filter(|expressions|expressions.len()<=64)
                    .ok_or_else(||ApiError::bad("Invalid/oversized VRM expression map"))?;
                for (name,expression) in expressions {
                    if name.is_empty() || name.len()>64 || !expression.is_object() {
                        return Err(ApiError::bad("Invalid VRM expression"));
                    }
                    for bind in array(&expression["morphTargetBinds"],256)? {
                        bind_count+=1;
                        let node=index(&bind["node"],nodes.len())?;
                        let mesh=index(&nodes[node]["mesh"],meshes.len())?;
                        index(&bind["index"],morph_count(&meshes[mesh]))?;
                        if bind.get("weight").is_some_and(|value|!scalar(value,0.0,1.0)) {
                            return Err(ApiError::bad("Invalid VRM expression weight"));
                        }
                    }
                    for bind in array(&expression["materialColorBinds"],256)? {
                        bind_count+=1;
                        index(&bind["material"],materials)?;
                        if !matches!(bind["type"].as_str(),Some("color"|"emissionColor"|"shadeColor"|"matcapColor"|"rimColor"|"outlineColor"))
                            || !(vector(&bind["targetValue"],3,8.0) || vector(&bind["targetValue"],4,8.0)) {
                            return Err(ApiError::bad("Invalid VRM material expression"));
                        }
                    }
                    for bind in array(&expression["textureTransformBinds"],256)? {
                        bind_count+=1;
                        index(&bind["material"],materials)?;
                        if bind.get("scale").is_some_and(|value|!vector(value,2,10000.0)) || bind.get("offset").is_some_and(|value|!vector(value,2,10000.0)) {
                            return Err(ApiError::bad("Invalid VRM texture transform expression"));
                        }
                    }
                    if bind_count>2048 { return Err(ApiError::bad("VRM expression bind budget exceeded")); }
                }
            }
        }
    }
    if let Some(spring)=doc.pointer("/extensions/VRMC_springBone") {
        let colliders=array(&spring["colliders"],64)?;
        for collider in colliders {
            index(&collider["node"],nodes.len())?;
            let shape=collider["shape"].as_object().filter(|shape|shape.len()==1)
                .ok_or_else(||ApiError::bad("Invalid VRM spring collider shape"))?;
            let (kind,shape)=shape.iter().next().unwrap();
            if !matches!(kind.as_str(),"sphere"|"capsule") || shape.get("offset").is_some_and(|value|!vector(value,3,10000.0))
                || shape.get("radius").is_some_and(|value|!scalar(value,0.0,10.0))
                || (kind=="capsule" && shape.get("tail").is_some_and(|value|!vector(value,3,10000.0))) {
                return Err(ApiError::bad("Invalid VRM spring collider parameters"));
            }
        }
        let groups=array(&spring["colliderGroups"],64)?;
        for group in groups {
            let mut unique=std::collections::HashSet::new();
            for reference in array(&group["colliders"],64)? {
                if !unique.insert(index(reference,colliders.len())?) {
                    return Err(ApiError::bad("Duplicate VRM collider group entry"));
                }
            }
        }
        let mut joint_count=0usize;
        let mut pairs=0usize;
        for chain in array(&spring["springs"],256)? {
            if let Some(center)=chain.get("center") { index(center,nodes.len())?; }
            let joints=array(&chain["joints"],256)?;
            if joints.is_empty() { return Err(ApiError::bad("VRM spring requires joints")); }
            joint_count+=joints.len();
            let mut unique=std::collections::HashSet::new();
            for joint in joints {
                if !unique.insert(index(&joint["node"],nodes.len())?) {
                    return Err(ApiError::bad("Duplicate VRM spring joint in chain"));
                }
                for (property,minimum,maximum) in [("hitRadius",0.0,10.0),("stiffness",0.0,100.0),("gravityPower",0.0,100.0),("dragForce",0.0,1.0)] {
                    if joint.get(property).is_some_and(|value|!scalar(value,minimum,maximum)) {
                        return Err(ApiError::bad("Invalid VRM spring joint parameter"));
                    }
                }
                if joint.get("gravityDir").is_some_and(|value|!vector(value,3,1.0)) {
                    return Err(ApiError::bad("Invalid VRM spring gravity direction"));
                }
            }
            let mut referenced=std::collections::HashSet::new();
            for reference in array(&chain["colliderGroups"],64)? {
                let group=index(reference,groups.len())?;
                if !referenced.insert(group) { return Err(ApiError::bad("Duplicate VRM spring collider group")); }
                pairs+=joints.len()*array(&groups[group]["colliders"],64)?.len();
            }
            if joint_count>256 || pairs>65536 {
                return Err(ApiError::bad("VRM spring joint/collider work budget exceeded"));
            }
        }
    }
    Ok(())
}

fn animations(doc: &Value, accessors: &[Value], nodes: &[Value]) -> ApiResult<()> {
    let mut keys=0u64;
    for animation in array(&doc["animations"],256)? {
        let samplers=array(&animation["samplers"],4096)?;
        for sampler in samplers {
            let input=&accessors[index(&sampler["input"],accessors.len())?];
            let output=&accessors[index(&sampler["output"],accessors.len())?];
            if !shape(input,"SCALAR",&[5126]) || output["componentType"].as_u64()!=Some(5126)
                || sampler.get("interpolation").is_some_and(|value|!matches!(value.as_str(),Some("LINEAR"|"STEP"|"CUBICSPLINE"))) {
                return Err(ApiError::bad("Invalid glTF animation sampler"));
            }
            keys=keys.checked_add(input["count"].as_u64().unwrap_or(0)).filter(|n|*n<=262144)
                .ok_or_else(||ApiError::bad("glTF animation key budget exceeded"))?;
        }
        for channel in array(&animation["channels"],4096)? {
            let sampler=&samplers[index(&channel["sampler"],samplers.len())?];
            let node=index(&channel["target"]["node"],nodes.len())?;
            let output=&accessors[index(&sampler["output"],accessors.len())?];
            let expected=match channel["target"]["path"].as_str() {
                Some("translation"|"scale")=>"VEC3", Some("rotation")=>"VEC4", Some("weights")=>"SCALAR",
                _=>return Err(ApiError::bad("Unsupported glTF animation target")),
            };
            if !shape(output,expected,&[5126]) { return Err(ApiError::bad("glTF animation output shape mismatch")); }
            let input=&accessors[index(&sampler["input"],accessors.len())?];
            let factor=if sampler["interpolation"]=="CUBICSPLINE" {3} else {1};
            let weights=if expected=="SCALAR" {
                let meshes=doc["meshes"].as_array().map(Vec::as_slice).unwrap_or_default();
                let mesh=index(&nodes[node]["mesh"],meshes.len())?;
                let count=morph_count(&meshes[mesh]);
                if count==0 { return Err(ApiError::bad("Weight animation requires supplied morph targets")); }
                count as u64
            } else {1};
            let minimum=input["count"].as_u64().unwrap_or(0).checked_mul(factor).and_then(|n|n.checked_mul(weights))
                .ok_or_else(||ApiError::bad("glTF animation count overflow"))?;
            if output["count"].as_u64()!=Some(minimum) {
                return Err(ApiError::bad("glTF animation input/output counts disagree"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn scene() -> Value {
        json!({"accessors":[{"type":"VEC3","componentType":5126,"count":3},
            {"type":"SCALAR","componentType":5123,"count":3}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
            "nodes":[{"mesh":0}]})
    }
    #[test]
    fn vrm_expression_and_spring_references_are_bounded() {
        let mut doc=scene();
        doc["meshes"][0]["primitives"][0]["targets"]=json!([{"POSITION":0}]);
        doc["extensions"]=json!({"VRMC_vrm":{"humanoid":{"humanBones":{"head":{"node":0}}},
            "expressions":{"preset":{"happy":{"morphTargetBinds":[{"node":0,"index":0,"weight":1}]}}}},
            "VRMC_springBone":{"colliders":[{"node":0,"shape":{"sphere":{}}}],
                "colliderGroups":[{"colliders":[0]}],"springs":[{"joints":[{"node":0}],"colliderGroups":[0]}]}});
        assert!(validate(&doc).is_ok());
        for (pointer,value) in [
            ("/extensions/VRMC_vrm/expressions/preset/happy/morphTargetBinds/0/index",json!(1)),
            ("/extensions/VRMC_vrm/humanoid/humanBones/head/node",json!(99)),
            ("/extensions/VRMC_springBone/springs/0/joints/0/node",json!(99)),
            ("/extensions/VRMC_springBone/colliderGroups/0/colliders/0",json!(99)),
            ("/extensions/VRMC_springBone/springs/0/joints",json!([{"node":0},{"node":0}])),
        ] {
            let mut bad=doc.clone();
            *bad.pointer_mut(pointer).unwrap()=value;
            assert!(validate(&bad).is_err(),"{pointer}");
        }
        doc["extensions"]["VRMC_springBone"]["colliders"][0]["shape"]["sphere"]["radius"]=json!(-1);
        assert!(validate(&doc).is_err());
    }
    #[test]
    fn scene_references_shapes_and_geometry_limits_are_validated() {
        assert!(validate(&scene()).is_ok());
        for (pointer,value) in [
            ("/nodes/0/mesh",json!(1)),
            ("/meshes/0/primitives/0/indices",json!(100)),
            ("/accessors/1/componentType",json!(5126)),
            ("/accessors/0/type",json!("VEC4")),
            ("/accessors/1/count",json!(4_000_001)),
            ("/meshes/0/primitives/0/mode",json!(7)),
        ] {
            let mut doc=scene();
            if let Some(slot)=doc.pointer_mut(pointer) { *slot=value; }
            else { doc["meshes"][0]["primitives"][0]["mode"]=value; }
            assert!(validate(&doc).is_err(),"{pointer}");
        }
        let mut doc=scene();
        doc["nodes"][0]["rotation"]=json!([0,0,0,0]);
        assert!(validate(&doc).is_err());
        doc["nodes"][0]["rotation"]=json!([0,0,0,1]);
        assert!(validate(&doc).is_ok());
        doc["skins"]=json!([{"joints":[0,0]}]);
        assert!(validate(&doc).is_err());
    }
    #[test]
    fn animation_and_extension_texture_references_cannot_escape_tables() {
        let mut doc=scene();
        doc["animations"]=json!([{"samplers":[{"input":1,"output":0}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}]);
        assert!(validate(&doc).is_err()); // time input is not float
        doc["accessors"][1]["componentType"]=json!(5126);
        doc["meshes"][0]["primitives"][0].as_object_mut().unwrap().remove("indices");
        assert!(validate(&doc).is_ok());
        doc["animations"][0]["channels"][0]["target"]["node"]=json!(99);
        assert!(validate(&doc).is_err());
        doc.as_object_mut().unwrap().remove("animations");
        doc["textures"]=json!([{"source":0,"extensions":{"EXT_texture_webp":{"source":1}}}]);
        doc["images"]=json!([{}]);
        assert!(validate(&doc).is_err());
    }
}
