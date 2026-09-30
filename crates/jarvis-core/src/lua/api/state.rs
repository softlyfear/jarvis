// State Lua API: persistent key-value storage per command

use mlua::{Lua, Table, Value};
use std::path::PathBuf;
use std::fs;
use std::collections::HashMap;

const STATE_FILE: &str = ".state.json";

pub fn register(lua: &Lua, jarvis: &Table, command_path: &PathBuf) -> mlua::Result<()> {
    let state = lua.create_table()?;
    let state_path = super::fs::resolve_path(command_path, STATE_FILE, crate::lua::SandboxLevel::Standard)?;
    
    // jarvis.state.get(key)
    let state_path_get = state_path.clone();
    let get_fn = lua.create_function(move |lua, key: String| {
        let data = load_state(&state_path_get);
        
        if let Some(value) = data.get(&key) {
            json_to_lua_value(lua, value.clone())
        } else {
            Ok(Value::Nil)
        }
    })?;
    state.set("get", get_fn)?;
    
    // jarvis.state.set(key, value)
    let state_path_set = state_path.clone();
    let set_fn = lua.create_function(move |lua, (key, value): (String, Value)| {
        let mut data = load_state(&state_path_set);
        
        let json_value = super::json::to_json(lua, value)?;
        data.insert(key, json_value);
        
        save_state(&state_path_set, &data)?;
        Ok(true)
    })?;
    state.set("set", set_fn)?;
    
    // jarvis.state.delete(key)
    let state_path_delete = state_path.clone();
    let delete_fn = lua.create_function(move |_, key: String| {
        let mut data = load_state(&state_path_delete);
        let existed = data.remove(&key).is_some();
        save_state(&state_path_delete, &data)?;
        Ok(existed)
    })?;
    state.set("delete", delete_fn)?;
    
    // jarvis.state.clear()
    let state_path_clear = state_path.clone();
    let clear_fn = lua.create_function(move |_, ()| {
        let data: HashMap<String, serde_json::Value> = HashMap::new();
        save_state(&state_path_clear, &data)?;
        Ok(true)
    })?;
    state.set("clear", clear_fn)?;
    
    // jarvis.state.keys()
    let state_path_keys = state_path.clone();
    let keys_fn = lua.create_function(move |lua, ()| {
        let data = load_state(&state_path_keys);
        let table = lua.create_table()?;
        
        for (i, key) in data.keys().enumerate() {
            table.set(i + 1, key.clone())?;
        }
        
        Ok(table)
    })?;
    state.set("keys", keys_fn)?;
    
    // jarvis.state.all()
    let state_path_all = state_path.clone();
    let all_fn = lua.create_function(move |lua, ()| {
        let data = load_state(&state_path_all);
        let table = lua.create_table()?;
        
        for (key, value) in data {
            let lua_value = json_to_lua_value(lua, value)?;
            table.set(key, lua_value)?;
        }
        
        Ok(table)
    })?;
    state.set("all", all_fn)?;
    
    jarvis.set("state", state)?;
    
    Ok(())
}

fn load_state(path: &PathBuf) -> HashMap<String, serde_json::Value> {
    if !path.exists() {
        return HashMap::new();
    }
    
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_state(path: &PathBuf, data: &HashMap<String, serde_json::Value>) -> mlua::Result<()> {
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
    
    crate::storage::atomic_write(path, json.as_bytes())
        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
    
    Ok(())
}

fn json_to_lua_value(lua: &Lua, json: serde_json::Value) -> mlua::Result<Value> {
    use serde_json::Value as JsonValue;
    
    match json {
        JsonValue::Null => Ok(Value::Nil),
        JsonValue::Bool(b) => Ok(Value::Boolean(b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        JsonValue::String(s) => Ok(Value::String(lua.create_string(&s)?)),
        JsonValue::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.into_iter().enumerate() {
                table.set(i + 1, json_to_lua_value(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
        JsonValue::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k, json_to_lua_value(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}
