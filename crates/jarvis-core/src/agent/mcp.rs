// Minimal MCP stdio transport: stdout contains JSON-RPC only, never application logs.
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};

pub fn dispatch(
    request: Value,
    invoke: &dyn Fn(&str, &Value) -> Result<Value, String>,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request["method"].as_str().unwrap_or("");
    if request["jsonrpc"] == "2.0" && id.is_none() && method.starts_with("notifications/") {
        return None;
    }
    let valid_id = id
        .as_ref()
        .is_some_and(|id| id.is_string() || id.is_i64() || id.is_u64());
    let id = if valid_id { id.unwrap() } else { Value::Null };
    let error =
        |code, message| json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}});
    if request["jsonrpc"] != "2.0"
        || !valid_id
        || method.is_empty()
        || request.get("params").is_some_and(|p| !p.is_object())
    {
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
                Ok(v) => tool_result(v),
                Err(e) => text_result(&e, true),
            }
        }
        _ => return Some(error(-32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn text_result(text: &str, error: bool) -> Value {
    json!({"content":[{"type":"text","text":text}],"isError":error})
}
fn tool_result(value: Value) -> Value {
    if let Some(error) = value.get("error") {
        return text_result(error.as_str().unwrap_or("Некорректный результат"), true);
    }
    if let Some(url) = value.get("image_url") {
        return match url
            .as_str()
            .and_then(|url| url.strip_prefix("data:image/png;base64,"))
            .filter(|s| !s.is_empty())
        {
            Some(data) => {
                json!({"content":[{"type":"image","mimeType":"image/png","data":data}],"isError":false})
            }
            None => text_result("Некорректное изображение", true),
        };
    }
    match value.get("result").and_then(Value::as_str) {
        Some(text) => text_result(text, false),
        None => text_result("Некорректный результат", true),
    }
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
        assert!(dispatch(
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            &no_call
        )
        .is_none());
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

    #[test]
    fn malformed_request_ids_never_invoke_tools() {
        for id in [
            None,
            Some(Value::Null),
            Some(json!(false)),
            Some(json!({})),
            Some(json!(1.5)),
        ] {
            let mut request = json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"jarvis.open_app","arguments":{"name":"блокнот"}}});
            if let Some(id) = id {
                request["id"] = id;
            }
            let response = dispatch(request, &|_, _| panic!("invalid request executed")).unwrap();
            assert_eq!(response["error"]["code"], -32600);
            assert!(response["id"].is_null());
        }
    }

    #[test]
    fn malformed_bridge_results_are_reported_as_errors() {
        for value in [
            json!({}),
            json!({"result":true}),
            json!({"image_url":"https://example.com/"}),
            json!({"image_url":"data:image/png;base64,"}),
            json!({"result":"готово","error":"не выполнено"}),
        ] {
            assert_eq!(tool_result(value)["isError"], true);
        }
        assert_eq!(
            tool_result(json!({"image_url":"data:image/png;base64,iVBORw0KGgo="}))["isError"],
            false
        );
    }
}
