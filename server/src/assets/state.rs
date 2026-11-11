//! Schema-1 explicitly replicated state; scripts cannot create undeclared keys.
pub use crate::creator_state::REPLICATED_STATE_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateVariable {
    Boolean {
        default: bool,
    },
    Number {
        default: f64,
        min: f64,
        max: f64,
    },
    Integer {
        default: i64,
        min: i64,
        max: i64,
    },
    String {
        default: String,
        max_bytes: usize,
    },
    Vector3 {
        default: [f64; 3],
        min: f64,
        max: f64,
    },
}
impl StateVariable {
    pub fn lua_value(&self, value: mlua::Value) -> mlua::Result<Value> {
        fn number(value: mlua::Value) -> mlua::Result<f64> {
            let number = match value {
                mlua::Value::Integer(value) => value as f64,
                mlua::Value::Number(value) => value,
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "Expected a replicated number".into(),
                    ));
                }
            };
            if !number.is_finite() {
                return Err(mlua::Error::RuntimeError(
                    "Expected a finite replicated number".into(),
                ));
            }
            Ok(number)
        }
        // Decode by the declared flat shape before any generic deserialization.
        // Creator tables can be cyclic/deep; they must never reach a recursive
        // serde_json::Value traversal on the host's native stack.
        let value = match self {
            Self::Boolean { .. } => {
                let mlua::Value::Boolean(value) = value else {
                    return Err(mlua::Error::RuntimeError(
                        "Expected a replicated boolean".into(),
                    ));
                };
                json!(value)
            }
            Self::String { max_bytes, .. } => {
                let mlua::Value::String(value) = value else {
                    return Err(mlua::Error::RuntimeError(
                        "Expected a replicated string".into(),
                    ));
                };
                if value.as_bytes().len() > *max_bytes {
                    return Err(mlua::Error::RuntimeError(
                        "Replicated string exceeds declared limit".into(),
                    ));
                }
                json!(value.to_str()?.to_string())
            }
            Self::Number { .. } | Self::Integer { .. } => json!(number(value)?),
            Self::Vector3 { .. } => {
                let mlua::Value::Table(value) = value else {
                    return Err(mlua::Error::RuntimeError(
                        "Expected a replicated three-component vector".into(),
                    ));
                };
                if value.raw_len() != 3 {
                    return Err(mlua::Error::RuntimeError(
                        "Expected three replicated vector components".into(),
                    ));
                }
                json!([
                    number(value.raw_get(1)?)?,
                    number(value.raw_get(2)?)?,
                    number(value.raw_get(3)?)?
                ])
            }
        };
        self.checked_value(value).ok_or_else(|| {
            mlua::Error::RuntimeError("Replicated value violates its declaration".into())
        })
    }
    pub fn default_value(&self) -> Value {
        match self {
            Self::Boolean { default } => json!(default),
            Self::Number { default, .. } => json!(default),
            Self::Integer { default, .. } => json!(default),
            Self::String { default, .. } => json!(default),
            Self::Vector3 { default, .. } => json!(default),
        }
    }
    pub fn valid(&self) -> bool {
        let bounds = match self {
            Self::Number { min, max, .. } | Self::Vector3 { min, max, .. } => {
                min.is_finite() && max.is_finite() && min <= max && *min >= -1e9 && *max <= 1e9
            }
            Self::Integer { min, max, .. } => {
                min <= max && *min >= -1_000_000_000 && *max <= 1_000_000_000
            }
            Self::String { max_bytes, .. } => (1..=4096).contains(max_bytes),
            Self::Boolean { .. } => true,
        };
        let default = self.default_value();
        bounds && self.accepts(&default) && default.to_string().len() <= 8192
    }
    pub fn accepts(&self, value: &Value) -> bool {
        match self {
            Self::Boolean { .. } => value.is_boolean(),
            Self::Number { min, max, .. } => in_range(value, *min, *max),
            // Luau's JSON bridge represents integral numbers as either JSON
            // integers or floats. Require exact integer values, not coercion.
            Self::Integer { min, max, .. } => {
                in_range(value, *min as f64, *max as f64)
                    && value.as_f64().is_some_and(|n| n.fract() == 0.0)
            }
            Self::String { max_bytes, .. } => value.as_str().is_some_and(|s| s.len() <= *max_bytes),
            Self::Vector3 { min, max, .. } => value
                .as_array()
                .is_some_and(|v| v.len() == 3 && v.iter().all(|n| in_range(n, *min, *max))),
        }
    }
    pub fn checked_value(&self, value: Value) -> Option<Value> {
        if !self.accepts(&value) {
            return None;
        }
        Some(if matches!(self, Self::Integer { .. }) {
            json!(value.as_f64()? as i64)
        } else {
            value
        })
    }
}
fn in_range(value: &Value, min: f64, max: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|n| n.is_finite() && (min..=max).contains(&n))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lua_values_are_decoded_by_shape_without_recursive_table_walks() {
        let lua = mlua::Lua::new();
        let vector = StateVariable::Vector3 {
            default: [0.0; 3],
            min: -10.0,
            max: 10.0,
        };
        let number = StateVariable::Number {
            default: 0.0,
            min: -10.0,
            max: 10.0,
        };
        for source in [
            "local t={}; t[1]=t; t[2]=0; t[3]=0; return t",
            "local t={}; local p=t; for i=1,10000 do local n={}; p[1]=n; p=n end; t[2]=0; t[3]=0; return t",
        ] {
            let value: mlua::Value = lua.load(source).eval().unwrap();
            assert!(vector.lua_value(value.clone()).is_err());
            assert!(number.lua_value(value).is_err());
        }
        let value = lua
            .load("return setmetatable({1,2,3},{__index=function() error('must not run') end})")
            .eval()
            .unwrap();
        assert_eq!(vector.lua_value(value).unwrap(), json!([1.0, 2.0, 3.0]));
        let integer = StateVariable::Integer {
            default: 0,
            min: 0,
            max: 10,
        };
        assert_eq!(
            integer.lua_value(mlua::Value::Number(2.0)).unwrap(),
            json!(2)
        );
        assert!(integer.lua_value(mlua::Value::Number(2.5)).is_err());
        assert!(
            number
                .lua_value(mlua::Value::Number(f64::INFINITY))
                .is_err()
        );
        let boolean = StateVariable::Boolean { default: false };
        assert!(boolean.lua_value(mlua::Value::Integer(1)).is_err());
        let string = StateVariable::String {
            default: String::new(),
            max_bytes: 3,
        };
        assert!(
            string
                .lua_value(mlua::Value::String(lua.create_string([255]).unwrap()))
                .is_err()
        );
        assert_eq!(
            string
                .lua_value(mlua::Value::String(lua.create_string("猫").unwrap()))
                .unwrap(),
            json!("猫")
        );
    }
    #[test]
    fn declarations_and_values_are_bounded_and_typed() {
        for (definition, valid, invalid) in [
            (
                json!({"type":"boolean","default":false}),
                json!(true),
                json!(1),
            ),
            (
                json!({"type":"number","default":0,"min":-1,"max":1}),
                json!(0.5),
                json!(2),
            ),
            (
                json!({"type":"integer","default":0,"min":0,"max":10}),
                json!(2.0),
                json!(1.5),
            ),
            (
                json!({"type":"string","default":"","max_bytes":3}),
                json!("猫"),
                json!("猫猫"),
            ),
            (
                json!({"type":"vector3","default":[0,0,0],"min":-10,"max":10}),
                json!([1, 2, 3]),
                json!([1, 2, 30]),
            ),
        ] {
            let declaration: StateVariable = serde_json::from_value(definition).unwrap();
            assert!(declaration.valid());
            assert!(declaration.accepts(&valid));
            assert!(!declaration.accepts(&invalid));
            assert!(!declaration.accepts(&Value::Null));
        }
        for invalid in [
            json!({"type":"integer","default":11,"min":0,"max":10}),
            json!({"type":"number","default":0,"min":1,"max":-1}),
            json!({"type":"string","default":"bad","max_bytes":2}),
            json!({"type":"vector3","default":[0,0,0],"min":-1e10,"max":1e10}),
        ] {
            assert!(
                !serde_json::from_value::<StateVariable>(invalid)
                    .unwrap()
                    .valid()
            );
        }
        assert!(
            serde_json::from_value::<StateVariable>(
                json!({"type":"boolean","default":false,"extra":true})
            )
            .is_err()
        );
    }
}
