use std::collections::HashSet;
use mlua::{Lua, LuaSerdeExt, DeserializeOptions, Value};

pub(crate) fn to_json(lua: &Lua, value: Value) -> mlua::Result<serde_json::Value> {
    validate(&value, 0, &mut HashSet::new())?;
    lua.from_value_with(value, DeserializeOptions::new().detect_mixed_tables(true))
}

fn validate(value: &Value, depth: usize, stack: &mut HashSet<*const std::ffi::c_void>) -> mlua::Result<()> {
    if depth > 64 { return Err(mlua::Error::runtime("JSON nesting exceeds 64 levels")); }
    if let Value::Table(table) = value {
        let id = table.to_pointer();
        if !stack.insert(id) { return Err(mlua::Error::runtime("Recursive table cannot be JSON")); }
        for pair in table.clone().pairs::<Value, Value>() {
            let (_, value) = pair?;
            validate(&value, depth + 1, stack)?;
        }
        stack.remove(&id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrays_are_ordered_and_recursive_or_deep_tables_are_errors() {
        let lua = Lua::new();
        let value = lua.load("local t = {}; t[3] = 'c'; t[1] = 'a'; t[2] = 'b'; return t").eval().unwrap();
        assert_eq!(to_json(&lua, value).unwrap(), serde_json::json!(["a", "b", "c"]));
        for source in ["local t = {}; t.self = t; return t", "local t = {}; local p = t; for i=1,100 do p.next = {}; p = p.next end; return t"] {
            assert!(to_json(&lua, lua.load(source).eval().unwrap()).is_err());
        }
        let value = lua.load("local a={x=1}; return {a=a, b=a}").eval().unwrap();
        assert_eq!(to_json(&lua, value).unwrap(), serde_json::json!({"a":{"x":1}, "b":{"x":1}}));
    }
}
