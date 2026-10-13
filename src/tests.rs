use crate::ai::tools::{execute, parse_tool_calls, strip_tool_blocks, ToolCall};
use crate::runtime::Runtime;
use crate::scene::{MeshKind, Scene};
use crate::templates::Template;
use serde_json::json;
use std::collections::HashSet;

#[test]
fn parses_tool_blocks() {
    let text = "Building now.\n```tool\n{\"tool\":\"create_entity\",\"args\":{\"name\":\"A\"}}\n```\n```json\n[{\"tool\":\"get_scene\"},{\"name\":\"delete_entity\",\"arguments\":\"{\\\"target\\\":\\\"A\\\"}\"}]\n```";
    let calls = parse_tool_calls(text);
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].name, "create_entity");
    assert_eq!(calls[2].args["target"], "A");
    assert_eq!(strip_tool_blocks(text).lines().next(), Some("Building now."));
}

#[test]
fn bare_json_line_is_a_call() {
    let calls = parse_tool_calls("ok\n{\"tool\": \"get_scene\", \"args\": {}}\n");
    assert_eq!(calls.len(), 1);
}

#[test]
fn tools_edit_scene() {
    let mut s = Scene::starter();
    let call = |name: &str, args| ToolCall { name: name.into(), args };
    execute(&mut s, &call("create_many", json!({"entities": [
        {"name": "Wall", "mesh": "cube", "position": [1, 2, 3], "scale": [4, 1, 1], "color": "red"},
        {"name": "Ball", "mesh": "sphere", "rigidbody": {"mass": 2.0}}
    ]}))).unwrap();
    let wall = s.get(s.find("Wall").unwrap()).unwrap();
    assert_eq!(wall.transform.position.y, 2.0);
    assert!(wall.color[0] > 0.8);
    assert!(s.get(s.find("ball").unwrap()).unwrap().rigidbody.is_some());
    execute(&mut s, &call("duplicate_entity", json!({"target": "Wall", "count": 3, "offset": [0, 0, -2]}))).unwrap();
    assert_eq!(s.entities.iter().filter(|e| e.name.starts_with("Wall")).count(), 4);
    assert!(execute(&mut s, &call("set_script", json!({"target": "Ball", "code": "fn update(dt) { this.x += "}))).is_err());
    execute(&mut s, &call("set_script", json!({"target": "Ball", "code": "fn update(dt) { this.x += 1.0 * dt; }"}))).unwrap();
    execute(&mut s, &call("add_camera", json!({"follow": "Ball", "position": [0, 5, 10]}))).unwrap();
    execute(&mut s, &call("delete_entity", json!({"target": "Wall"}))).unwrap();
    assert!(s.find("Wall").is_none());
    assert!(execute(&mut s, &call("nope", json!({}))).is_err());
}

#[test]
fn platformer_plays() {
    let scene = Template::Platformer.build();
    let (mut rt, errors) = Runtime::new(scene);
    assert!(errors.is_empty(), "{errors:?}");
    // Fall onto the ground and settle.
    for _ in 0..120 {
        let e = rt.step(1.0 / 60.0);
        assert!(e.is_empty(), "{e:?}");
    }
    let player = rt.scene.find("Player").unwrap();
    let p = rt.scene.get(player).unwrap();
    assert!(p.rigidbody.as_ref().unwrap().grounded, "player should be grounded, y={}", p.transform.position.y);
    assert!((p.transform.position.y - 0.8).abs() < 0.05, "y={}", p.transform.position.y);
    assert!(rt.hud().contains("Coins: 0"), "{}", rt.hud());
    // Walk right.
    let x0 = p.transform.position.x;
    let down: HashSet<String> = ["D".to_string()].into();
    rt.set_input(down, HashSet::new());
    for _ in 0..30 {
        rt.step(1.0 / 60.0);
    }
    let x1 = rt.scene.get(player).unwrap().transform.position.x;
    assert!(x1 > x0 + 2.0, "moved {x0} -> {x1}");
    // Teleport onto a coin → collected.
    let coin = rt.scene.find("Coin 1").unwrap();
    let cp = rt.scene.get(coin).unwrap().transform.position;
    rt.scene.get_mut(player).unwrap().transform.position = cp;
    rt.set_input(HashSet::new(), HashSet::new());
    rt.step(1.0 / 60.0);
    rt.step(1.0 / 60.0);
    assert!(rt.scene.find("Coin 1").is_none());
    assert!(rt.hud().contains("Coins: 1"), "{}", rt.hud());
}

#[test]
fn scene_roundtrip() {
    let s = Template::Platformer.build();
    let txt = serde_json::to_string(&s).unwrap();
    let back: Scene = serde_json::from_str(&txt).unwrap();
    assert_eq!(back.entities.len(), s.entities.len());
    assert_eq!(MeshKind::parse("Box"), MeshKind::Cube);
}

/// Minimal OpenAI-compatible SSE server: plans, emits tool blocks, writes a script, then summarizes.
fn mock_llm() -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.to_ascii_lowercase().starts_with("content-length:") {
                    len = line[15..].trim().parse().unwrap();
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let msgs = req["messages"].as_array().unwrap();
            let sys = msgs[0]["content"].as_str().unwrap();
            let last = msgs.last().unwrap()["content"].as_str().unwrap();
            let text = if sys.contains("PLANNER") {
                "1. Build a red tower\n2. Add a spinning orb".to_string()
            } else if sys.contains("CODER") {
                "```rhai\nfn update(dt) { this.ry += 90.0 * dt; }\n```".to_string()
            } else if last.contains("Tool results") {
                "Done: tower and orb are ready.".to_string()
            } else {
                let calls = [
                    json!({"tool":"create_many","args":{"entities":[
                        {"name":"Tower","mesh":"cylinder","position":[3,2,-2],"scale":[1.5,4,1.5],"color":"#e53935"},
                        {"name":"Orb","mesh":"sphere","position":[3,4.6,-2],"color":"gold"}]}}),
                    json!({"tool":"ask_coder","args":{"target":"Orb","goal":"spin"}}),
                ];
                calls.iter().map(|c| format!("```tool\n{c}\n```\n")).collect()
            };
            let mut out = String::from("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
            for chunk in text.as_bytes().chunks(10) {
                let piece = String::from_utf8_lossy(chunk);
                out.push_str(&format!("data: {}\n\n", json!({"model":"mock-1","choices":[{"delta":{"content":piece}}]})));
            }
            out.push_str("data: [DONE]\n\n");
            let _ = stream.write_all(out.as_bytes());
        }
    });
    format!("http://{addr}/v1")
}

#[test]
fn agents_build_scene_end_to_end() {
    use crate::ai::agent::{self, AgentEvent, Job, Mode};
    let mut settings = crate::config::Settings::default();
    settings.ollama_url = mock_llm();
    settings.selected_model = "ollama:mock".into();
    settings.coder_model = String::new();
    settings.use_planner = true;
    let mut scene = Scene::starter();
    let job = Job {
        prompt: "Bir kule yap".into(),
        mode: Mode::Agent,
        history: vec![],
        scene_json: scene.summary(),
        selected: None,
        settings,
        openrouter_key: String::new(),
    };
    let h = agent::spawn(job, egui::Context::default());
    let mut roles = vec![];
    let mut final_text = String::new();
    loop {
        match h.rx.recv_timeout(std::time::Duration::from_secs(10)).expect("agent timed out") {
            AgentEvent::Start { role, .. } => {
                roles.push(role.label());
                final_text.clear();
            }
            AgentEvent::Delta { text } => final_text.push_str(&text),
            AgentEvent::Tool { call, reply } => {
                let _ = reply.send(execute(&mut scene, &call).map(|o| o.result));
            }
            AgentEvent::Error(e) => panic!("agent error: {e}"),
            AgentEvent::Done => break,
            _ => {}
        }
    }
    assert_eq!(roles, ["Planner", "Builder", "Coder", "Builder"]);
    assert!(final_text.contains("Done"));
    let orb = scene.get(scene.find("Orb").unwrap()).unwrap();
    assert!(orb.script.contains("this.ry"));
    assert!(scene.find("Tower").is_some());
}
