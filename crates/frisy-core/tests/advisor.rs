//! Provider tests against a local mock server: no real keys, no network.

use frisy_core::advisor::{self, Message};
use frisy_core::api::Api;
use frisy_core::settings::{self, Settings};
use serde_json::json;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// Serves canned responses and records each request as "METHOD url | auth | body".
fn mock(routes: Vec<(&'static str, u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr().to_ip().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
            let auth = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization") || h.field.equiv("x-api-key"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();
            log.lock().unwrap().push(format!("{} {} | {auth} | {body}", req.method(), req.url()));
            let (status, text) = routes.iter().find(|r| r.0 == req.url()).map(|r| (r.1, r.2)).unwrap_or((404, "{}"));
            let _ = req.respond(tiny_http::Response::from_string(text).with_status_code(status));
        }
    });
    (base, seen)
}

fn chat() -> Vec<Message> {
    vec![
        Message { role: "system".into(), text: "be brief".into() },
        Message { role: "user".into(), text: "hello".into() },
    ]
}

fn collect(run: impl FnOnce(&mut dyn FnMut(&str)) -> Result<(), String>) -> Result<String, String> {
    let mut out = String::new();
    run(&mut |c| out.push_str(c))?;
    Ok(out)
}

#[test]
fn anthropic_stream_is_parsed_and_system_prompt_is_top_level() {
    let sse = "event: message_start\ndata: {\"type\":\"message_start\"}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Move \"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"it.\"}}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let (base, seen) = mock(vec![("/v1/messages", 200, Box::leak(sse.to_string().into_boxed_str()))]);
    let stop = AtomicBool::new(false);
    let text = collect(|f| advisor::stream_anthropic(&base, "sk-test", &chat(), "claude-opus-5-5", &stop, f)).unwrap();
    assert_eq!(text, "Move it.");
    let req = seen.lock().unwrap()[0].clone();
    assert!(req.starts_with("POST /v1/messages | sk-test | "));
    let body: serde_json::Value = serde_json::from_str(req.splitn(3, " | ").nth(2).unwrap()).unwrap();
    assert_eq!(body["system"], "be brief");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["model"], "claude-opus-5-5");
}

#[test]
fn anthropic_refusal_and_bad_key_are_reported() {
    let refusal = "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"}}\n\n";
    let (base, _) = mock(vec![("/v1/messages", 200, refusal)]);
    let stop = AtomicBool::new(false);
    let err = collect(|f| advisor::stream_anthropic(&base, "k", &chat(), "m", &stop, f)).unwrap_err();
    assert!(err.contains("declined"));

    let (base, _) = mock(vec![("/v1/messages", 401, "{\"error\":{\"message\":\"invalid x-api-key\"}}")]);
    let err = collect(|f| advisor::stream_anthropic(&base, "k", &chat(), "m", &stop, f)).unwrap_err();
    assert!(err.contains("rejected the API key") && err.contains("invalid x-api-key"), "{err}");
}

#[test]
fn openai_style_stream_is_parsed() {
    let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"Use \"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"the NAS.\"}}]}\n\n\
data: [DONE]\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"ignored\"}}]}\n\n";
    let (base, seen) = mock(vec![("/v1/chat/completions", 200, sse)]);
    let stop = AtomicBool::new(false);
    let text = collect(|f| advisor::stream_openai("AgenticWork", &base, "awc_test", &chat(), "auto", &stop, f)).unwrap();
    assert_eq!(text, "Use the NAS.");
    let req = seen.lock().unwrap()[0].clone();
    assert!(req.contains("| Bearer awc_test |") && req.contains("\"role\":\"system\""));
}

#[test]
fn ollama_stream_is_parsed_from_a_custom_host() {
    let lines = "{\"message\":{\"content\":\"Hi \"},\"done\":false}\n{\"message\":{\"content\":\"there\"},\"done\":true}\n";
    let tags = "{\"models\":[{\"name\":\"big:latest\",\"size\":9000000000,\"capabilities\":[\"completion\"]},\
{\"name\":\"tiny:latest\",\"size\":1000000000},{\"name\":\"embed:latest\",\"size\":5000000000,\"capabilities\":[\"embedding\"]}]}";
    let (base, _) = mock(vec![("/api/chat", 200, lines), ("/api/tags", 200, tags)]);
    let stop = AtomicBool::new(false);
    assert_eq!(collect(|f| advisor::stream_ollama(&base, &chat(), "big:latest", &stop, f)).unwrap(), "Hi there");
    assert_eq!(advisor::ollama_models(&base), ["big:latest"]);
}

#[test]
fn backends_follow_the_settings() {
    let tags = "{\"models\":[{\"name\":\"big:latest\",\"size\":9000000000}]}";
    let models = "{\"data\":[{\"id\":\"auto\",\"model_type\":\"chat\"},{\"id\":\"canvas\",\"model_type\":\"image\"},{\"id\":\"fast\"}]}";
    let (base, _) = mock(vec![("/api/tags", 200, tags), ("/v1/models", 200, models)]);

    // Nothing reachable and no keys: no backends, so the UI hides the advisor.
    let mut s = Settings { ollama_host: "http://127.0.0.1:9".into(), copilot_enabled: false, ..Settings::default() };
    assert!(advisor::backends(&s).is_empty());

    // Ollama on another address.
    s.ollama_host = base.clone();
    let found = advisor::backends(&s);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].kind.as_str(), found[0].model.as_str(), found[0].remote), ("ollama", "big:latest", false));

    // An AgenticWork key adds its chat models, default first, marked as leaving this computer.
    s.agenticwork_url = base.clone();
    s.agenticwork_key = "awc_test".into();
    let found = advisor::backends(&s);
    let aw: Vec<&str> = found.iter().filter(|b| b.kind == "agenticwork").map(|b| b.model.as_str()).collect();
    assert_eq!(aw, ["auto", "fast"]);
    assert!(found.iter().filter(|b| b.kind == "agenticwork").all(|b| b.remote));

    // Turned off: nothing, whatever is configured.
    s.advisor_enabled = false;
    assert!(advisor::backends(&s).is_empty());
}

#[test]
fn settings_are_saved_privately_and_keys_never_reach_the_ui() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FRISYDISK_CONFIG_DIR", dir.path());
    assert_eq!(Settings::load(), Settings::default());

    let api = Api::new();
    let shown = api
        .call("set_settings", &json!({ "ollama_host": "nas.local:11434/", "agenticwork_url": "https://chat.example.com/v1/", "agenticwork_key": " awc_secret ", "anthropic_key": "sk-ant-secret", "copilot_url": "127.0.0.1:62783/", "copilot_token": "cop_secret" }))
        .unwrap();
    assert_eq!(shown["ollama_host"], "http://nas.local:11434");
    assert_eq!(shown["agenticwork_url"], "https://chat.example.com");
    assert_eq!((shown["has_agenticwork_key"].clone(), shown["has_anthropic_key"].clone(), shown["has_openai_key"].clone()), (json!(true), json!(true), json!(false)));
    assert_eq!((shown["copilot_enabled"].clone(), shown["copilot_url"].clone(), shown["has_copilot_token"].clone()), (json!(true), json!("http://127.0.0.1:62783"), json!(true)));
    assert!(!shown.to_string().contains("secret"));
    assert!(!api.call("settings", &json!({})).unwrap().to_string().contains("secret"));

    let saved = Settings::load();
    assert_eq!((saved.agenticwork_key.as_str(), saved.anthropic_key.as_str(), saved.copilot_token.as_str()), ("awc_secret", "sk-ant-secret", "cop_secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(settings::path()).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // Leaving a key out keeps it; an empty string clears it; an empty host goes back to the default.
    api.call("set_settings", &json!({ "advisor_enabled": false, "anthropic_key": "", "ollama_host": "", "copilot_enabled": false })).unwrap();
    let saved = Settings::load();
    assert_eq!((saved.copilot_token.as_str(), saved.copilot_enabled), ("cop_secret", false));
    assert_eq!((saved.agenticwork_key.as_str(), saved.anthropic_key.as_str(), saved.advisor_enabled), ("awc_secret", "", false));
    assert_eq!(saved.ollama_host, settings::DEFAULT_OLLAMA);
    assert!(api.call("backends", &json!({})).unwrap().as_array().unwrap().is_empty());
    assert!(api.call("advisor_ask", &json!({ "backend": { "kind": "ollama", "model": "m", "label": "m" } })).is_err());

    assert!(settings::is_local("http://127.0.0.1:11434") && settings::is_local("http://localhost:1"));
    assert!(!settings::is_local("http://nas.local:11434") && !settings::is_local("https://api.example.com"));
}

const COPILOT_MODELS: &str = "{\"data\":[{\"id\":\"copilot/gpt-5\",\"display_name\":\"GPT-5\"},{\"id\":\"copilot/claude-sonnet-5\",\"display_name\":\"Claude Sonnet 5\"},{\"id\":\"other/model\"}]}";
const ANTHROPIC_OK: &str = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n";

/// Copilot settings that skip the lockfile, so these tests do not depend on
/// whichever VS Code windows happen to be open.
fn copilot_at(base: &str, token: &str) -> Settings {
    Settings { ollama_host: "http://127.0.0.1:9".into(), copilot_url: base.into(), copilot_token: token.into(), ..Settings::default() }
}

#[test]
fn copilot_bridge_is_found_from_a_live_lockfile() {
    let (base, _) = mock(vec![("/v1/models", 200, COPILOT_MODELS)]);
    let port = base.rsplit(':').next().unwrap().to_string();
    let dir = tempfile::tempdir().unwrap();
    let lm = dir.path().join("lm");
    std::fs::create_dir_all(&lm).unwrap();
    std::env::set_var("AGENTICODE_CONFIG_DIR", dir.path());
    let me = std::process::id();
    let write = |name: &str, v: serde_json::Value| std::fs::write(lm.join(name), v.to_string()).unwrap();
    write("1.json", json!({ "port": 1, "token": "dead", "pid": 999_999_999u64, "ideName": "Visual Studio Code", "api": "anthropic-messages" }));
    write("2.json", json!({ "port": 2, "token": "other", "pid": me, "ideName": "Visual Studio Code", "api": "openai" }));
    let s = Settings { ollama_host: "http://127.0.0.1:9".into(), ..Settings::default() };
    assert_eq!(advisor::copilot_bridge(&s), None);
    assert!(advisor::backends(&s).is_empty());

    write(&format!("{port}.json"), json!({ "port": port.parse::<u64>().unwrap(), "token": "tok", "pid": me, "ideName": "Visual Studio Code", "api": "anthropic-messages" }));
    assert_eq!(advisor::copilot_bridge(&s), Some((base.clone(), "tok".into())));
    let found = advisor::backends(&s);
    let models: Vec<&str> = found.iter().map(|b| b.model.as_str()).collect();
    assert_eq!(models, ["copilot/claude-sonnet-5", "copilot/gpt-5"]);
    assert_eq!(found[0].label, "Copilot: claude-sonnet-5");
    assert!(found.iter().all(|b| b.kind == "copilot" && b.remote));

    // Turned off: not offered even with a live bridge.
    assert!(advisor::backends(&Settings { copilot_enabled: false, ..s }).is_empty());
    std::env::remove_var("AGENTICODE_CONFIG_DIR");
}

#[test]
fn copilot_streams_through_the_bridge_like_anthropic() {
    let (base, seen) = mock(vec![("/v1/messages", 200, ANTHROPIC_OK)]);
    let s = copilot_at(&base, "tok");
    let b = advisor::Backend { kind: "copilot".into(), model: "copilot/claude-sonnet-5".into(), label: "Copilot".into(), remote: true };
    let stop = AtomicBool::new(false);
    assert_eq!(collect(|f| advisor::stream(&s, &chat(), &b, &stop, f)).unwrap(), "ok");
    let req = seen.lock().unwrap()[0].clone();
    assert!(req.starts_with("POST /v1/messages | tok | "), "{req}");
    let body: serde_json::Value = serde_json::from_str(req.splitn(3, " | ").nth(2).unwrap()).unwrap();
    assert_eq!((body["system"].as_str(), body["model"].as_str(), body["stream"].as_bool()), (Some("be brief"), Some("copilot/claude-sonnet-5"), Some(true)));
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn test_provider_checks_copilot() {
    let (base, seen) = mock(vec![("/v1/models", 200, COPILOT_MODELS), ("/v1/messages", 200, ANTHROPIC_OK)]);
    let r = advisor::test_provider(&copilot_at(&base, "tok"), "copilot");
    assert!(r.ok, "{r:?}");
    assert_eq!((r.model.as_str(), r.models), ("copilot/claude-sonnet-5", 2));
    assert!(seen.lock().unwrap().iter().any(|l| l.contains("Reply with exactly: ok")));

    let (base, _) = mock(vec![("/v1/models", 401, "{\"error\":{\"message\":\"bad token\"}}")]);
    let r = advisor::test_provider(&copilot_at(&base, "nope"), "copilot");
    let err = r.error.unwrap();
    assert!(!r.ok && err.contains("Copilot rejected the API key (HTTP 401)") && err.contains("bad token"), "{err}");

    let r = advisor::test_provider(&copilot_at("http://127.0.0.1:9", "tok"), "copilot");
    assert!(r.error.unwrap().contains("Could not reach Copilot"));
}

#[test]
fn test_provider_checks_ollama() {
    let tags = "{\"models\":[{\"name\":\"big:latest\",\"size\":9000000000}]}";
    let (base, _) = mock(vec![("/api/tags", 200, tags), ("/api/chat", 200, "{\"message\":{\"content\":\"ok\"},\"done\":true}\n")]);
    let r = advisor::test_provider(&Settings { ollama_host: base, ..Settings::default() }, "ollama");
    assert!(r.ok, "{r:?}");
    assert_eq!((r.model.as_str(), r.models), ("big:latest", 1));

    let r = advisor::test_provider(&Settings { ollama_host: "http://127.0.0.1:9".into(), ..Settings::default() }, "ollama");
    assert!(!r.ok && r.error.unwrap().contains("Could not reach Ollama"));

    let (base, _) = mock(vec![("/api/tags", 200, "{\"models\":[]}")]);
    let r = advisor::test_provider(&Settings { ollama_host: base, ..Settings::default() }, "ollama");
    assert!(r.error.unwrap().contains("no chat models"));
}

#[test]
fn test_provider_checks_agenticwork_with_typed_values() {
    let models = "{\"data\":[{\"id\":\"fast\"},{\"id\":\"auto\",\"model_type\":\"chat\"}]}";
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
    let (base, seen) = mock(vec![("/v1/models", 200, models), ("/v1/chat/completions", 200, sse)]);
    let api = Api::new();
    // Values typed in the form are used without being saved.
    let r = api.call("test_provider", &json!({ "kind": "agenticwork", "settings": { "agenticwork_url": base, "agenticwork_key": "awc_test" } })).unwrap();
    assert_eq!((r["ok"].clone(), r["model"].clone(), r["models"].clone()), (json!(true), json!("auto"), json!(2)), "{r}");
    assert!(seen.lock().unwrap().iter().any(|l| l.starts_with("POST /v1/chat/completions | Bearer awc_test")));
    assert!(!r.to_string().contains("awc_test"));

    let (bad, _) = mock(vec![("/v1/models", 403, "{\"message\":\"forbidden\"}")]);
    let r = advisor::test_provider(&Settings { agenticwork_url: bad, agenticwork_key: "x".into(), ..Settings::default() }, "agenticwork");
    assert!(r.error.unwrap().contains("AgenticWork rejected the API key (HTTP 403)"));
    let r = advisor::test_provider(&Settings { agenticwork_url: "http://127.0.0.1:9".into(), agenticwork_key: "x".into(), ..Settings::default() }, "agenticwork");
    assert!(r.error.unwrap().contains("Could not reach AgenticWork"));
    assert!(advisor::test_provider(&Settings::default(), "agenticwork").error.unwrap().contains("first"));
}
