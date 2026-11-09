//! Fixed creator presentation properties; never arbitrary engine property access.
use serde_json::Value;

pub enum Shape {
    Color,
    Unit,
    Energy,
    Range,
    Boolean,
    OutlineWidth,
    Matcap,
}
pub fn shape(kind: &str, property: &str) -> Option<Shape> {
    match (kind, property) {
        ("material", "albedo" | "emission" | "outline_color") | ("light", "color") => Some(Shape::Color),
        ("material", "roughness" | "metallic" | "specular" | "rim" | "rim_tint" | "matcap_strength") => Some(Shape::Unit),
        ("material", "emission_energy") | ("light", "energy") => Some(Shape::Energy),
        ("light", "range") => Some(Shape::Range),
        ("light", "enabled" | "shadow") => Some(Shape::Boolean),
        ("material", "outline_width") => Some(Shape::OutlineWidth),
        ("material", "matcap") => Some(Shape::Matcap),
        ("zone", "reverb" | "echo" | "distortion" | "noise") => Some(Shape::Unit),
        ("zone", "compression") => Some(Shape::Boolean),
        _ => None,
    }
}

pub fn validate(kind: &str, property: &str, value: &Value) -> Result<(), &'static str> {
    let color = || {
        value.as_array().is_some_and(|items| {
            items.len() == 3
                && items.iter().all(|item| {
                    item.as_f64().is_some_and(|number| {
                        number.is_finite() && (0.0..=1.0).contains(&number)
                    })
                })
        })
    };
    let scalar = |maximum: f64| {
        value.as_f64().is_some_and(|number| {
            number.is_finite() && (0.0..=maximum).contains(&number)
        })
    };
    let valid = match shape(kind, property) {
        Some(Shape::Color) => color(),
        Some(Shape::Unit) => scalar(1.0),
        Some(Shape::Energy) => scalar(8.0),
        Some(Shape::Range) => scalar(100.0) && value.as_f64().is_some_and(|value| value >= 0.01),
        Some(Shape::Boolean) => value.is_boolean(),
        Some(Shape::OutlineWidth) => scalar(0.02),
        Some(Shape::Matcap) => matches!(value.as_str(), Some("none" | "pearl" | "metal")),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("Unsupported or invalid creator visual property")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_typed_bounded_presentation_properties_are_allowed() {
        for (kind,property,value) in [("material","albedo",json!([1,0.5,0])),("material","roughness",json!(0.5)),("material","matcap",json!("pearl")),("material","outline_width",json!(0.02)),("light","enabled",json!(false)),("light","energy",json!(8))] {
            assert!(validate(kind,property,&value).is_ok());
        }
        for (kind,property,value) in [("material","shader",json!("file:///tmp/x")),("light","script",json!("x")),("material","albedo",json!([1,2,3])),("light","enabled",json!(1)),("light","range",json!(101)),("material","roughness",json!(-1))] {
            assert!(validate(kind,property,&value).is_err());
        }
    }
    #[test]
    fn acoustic_noise_uses_only_bounded_numeric_amounts() {
        for value in [json!(0),json!(0.25),json!(1)] {
            assert!(validate("zone","noise",&value).is_ok());
        }
        for value in [json!(-0.01),json!(1.01),json!(true),json!("0.5"),json!([0.5]),Value::Null] {
            assert!(validate("zone","noise",&value).is_err());
        }
        assert!(validate("zone","bus",&json!("Master")).is_err());
    }
}
