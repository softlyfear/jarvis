// Minimal MCP stdio transport: stdout contains JSON-RPC only, never application logs.
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};

pub fn dispatch(
    request: Value,
    invoke: &dyn Fn(&str, &Value) -> Result<Value, String>,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request["method"].as_str().unwrap_or("");
    if id.is_none() && method.starts_with("notifications/") {
        return None;
    }
    let id = id.unwrap_or(Value::Null);
    let error =
        |code, message| json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}});
    if request["jsonrpc"] != "2.0" {
        return Some(error(-32600, "Invalid request"));
    }
    let result = match method {
        "initialize" => {
            let version = request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2025-11-25");
            let version =
                if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"].contains(&version) {
                    version
                } else {
                    "2025-11-25"
                };
            json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"jarvis-pc","version":crate::config::APP_VERSION.unwrap_or("local")}})
        }
        "ping" => json!({}),
        "tools/list" => {
            json!({"tools":super::openclaw::definitions().iter().map(|t|json!({"name":format!("jarvis.{}",t["function"]["name"].as_str().unwrap()),"description":t["function"]["description"],"inputSchema":t["function"]["parameters"]})).collect::<Vec<_>>()})
        }
        "tools/call" => {
            let Some(name) = request["params"]["name"].as_str() else {
                return Some(error(-32602, "Missing tool name"));
            };
            let args = request["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !args.is_object()
                || !super::openclaw::definitions()
                    .iter()
                    .any(|t| format!("jarvis.{}", t["function"]["name"].as_str().unwrap()) == name)
            {
                return Some(error(-32602, "Unknown tool or invalid arguments"));
            }
            match invoke(name, &args) {
                Ok(v) if v["image_url"].as_str().is_some() => {
                    let data = v["image_url"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("data:image/png;base64,")
                        .unwrap_or("");
                    json!({"content":[{"type":"image","mimeType":"image/png","data":data}],"isError":false})
                }
                Ok(v) => {
                    json!({"content":[{"type":"text","text":v.get("result").or_else(||v.get("error")).and_then(Value::as_str).unwrap_or("Некорректный результат")}],"isError":v.get("error").is_some()})
                }
                Err(e) => json!({"content":[{"type":"text","text":e}],"isError":true}),
            }
        }
        _ => return Some(error(-32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

pub fn run(input: impl BufRead, mut output: impl Write) -> Result<(), String> {
    let mut input = input;
    loop {
        let mut line = Vec::new();
        if Read::take(&mut input, 64 * 1024 + 1)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        if line.len() > 64 * 1024 {
            return Err("MCP request too large".into());
        }
        let result = match serde_json::from_slice(&line) {
            Ok(v) => dispatch(v, &super::bridge::invoke),
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
        };
        if let Some(v) = result {
            writeln!(output, "{}", v).map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initialize_list_call_and_notifications() {
        let no_call = |_: &str, _: &Value| panic!("unexpected action");
        let init=dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),&no_call).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "jarvis-pc");
        assert!(dispatch(json!({"method":"notifications/initialized"}), &no_call).is_none());
        let list = dispatch(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            &no_call,
        )
        .unwrap();
        assert!(list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "jarvis.open_app"));
        let bad = dispatch(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"jarvis.shell"}}),
            &no_call,
        )
        .unwrap();
        assert_eq!(bad["error"]["code"], -32602);
        let ok=dispatch(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"jarvis.open_app","arguments":{"name":"блокнот"}}}),&|name,args|{assert_eq!(name,"jarvis.open_app");assert_eq!(args["name"],"блокнот");Ok(json!({"result":"готово"}))}).unwrap();
        assert_eq!(ok["result"]["isError"], false);
        let err=dispatch(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"jarvis.open_app"}}),&|_,_|Err("ошибка".into())).unwrap();
        assert_eq!(err["result"]["isError"], true);
    }
}
