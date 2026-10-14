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
    assert_eq!(
        strip_tool_blocks(text).lines().next(),
        Some("Building now.")
    );
}

#[test]
fn bare_json_line_is_a_call() {
    let calls = parse_tool_calls("ok\n{\"tool\": \"get_scene\", \"args\": {}}\n");
    assert_eq!(calls.len(), 1);
}

#[test]
fn tools_edit_scene() {
    let mut s = Scene::starter();
    let call = |name: &str, args| ToolCall {
        name: name.into(),
        args,
    };
    execute(&mut s, &call("create_many", json!({"entities": [
        {"name": "Wall", "mesh": "cube", "position": [1, 2, 3], "scale": [4, 1, 1], "color": "red"},
        {"name": "Ball", "mesh": "sphere", "rigidbody": {"mass": 2.0}}
    ]}))).unwrap();
    let wall = s.get(s.find("Wall").unwrap()).unwrap();
    assert_eq!(wall.transform.position.y, 2.0);
    assert!(wall.color[0] > 0.8);
    assert!(s.get(s.find("ball").unwrap()).unwrap().rigidbody.is_some());
    execute(
        &mut s,
        &call(
            "duplicate_entity",
            json!({"target": "Wall", "count": 3, "offset": [0, 0, -2]}),
        ),
    )
    .unwrap();
    assert_eq!(
        s.entities
            .iter()
            .filter(|e| e.name.starts_with("Wall"))
            .count(),
        4
    );
    assert!(execute(
        &mut s,
        &call(
            "set_script",
            json!({"target": "Ball", "code": "fn update(dt) { this.x += "})
        )
    )
    .is_err());
    execute(
        &mut s,
        &call(
            "set_script",
            json!({"target": "Ball", "code": "fn update(dt) { this.x += 1.0 * dt; }"}),
        ),
    )
    .unwrap();
    execute(
        &mut s,
        &call(
            "add_camera",
            json!({"follow": "Ball", "position": [0, 5, 10]}),
        ),
    )
    .unwrap();
    execute(&mut s, &call("delete_entity", json!({"target": "Wall"}))).unwrap();
    assert!(s.find("Wall").is_none());
    assert!(execute(&mut s, &call("nope", json!({}))).is_err());
}

#[test]
fn platformer_plays() {
    let scene = Template::Platformer.build();
    let (mut rt, errors) = Runtime::new(scene, None);
    assert!(errors.is_empty(), "{errors:?}");
    // Fall onto the ground and settle.
    for _ in 0..120 {
        let e = rt.step(1.0 / 60.0);
        assert!(e.is_empty(), "{e:?}");
    }
    let player = rt.scene.find("Player").unwrap();
    let p = rt.scene.get(player).unwrap();
    assert!(
        p.rigidbody.as_ref().unwrap().grounded,
        "player should be grounded, y={}",
        p.transform.position.y
    );
    assert!(
        (p.transform.position.y - 0.8).abs() < 0.05,
        "y={}",
        p.transform.position.y
    );
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
                calls
                    .iter()
                    .map(|c| format!("```tool\n{c}\n```\n"))
                    .collect()
            };
            let mut out = String::from(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            );
            for chunk in text.as_bytes().chunks(10) {
                let piece = String::from_utf8_lossy(chunk);
                out.push_str(&format!(
                    "data: {}\n\n",
                    json!({"model":"mock-1","choices":[{"delta":{"content":piece}}]})
                ));
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
        mode: Mode::Planning,
        history: vec![],
        scene_json: scene.summary(),
        selected: None,
        settings,
        openrouter_key: String::new(),
        web: false,
        memory: crate::ai::memory::MemoryConfig {
            backend: crate::ai::memory::MemoryBackend::Off,
            ..Default::default()
        },
        session_id: "test".into(),
        summary: String::new(),
        project_root: std::env::temp_dir(),
        permission: crate::config::Permission::Auto,
    };
    let h = agent::spawn(job, egui::Context::default());
    let mut roles = vec![];
    let mut final_text = String::new();
    loop {
        match h
            .rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("agent timed out")
        {
            AgentEvent::Start { role, .. } => {
                roles.push(format!("{role:?}"));
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

#[test]
fn parses_search_results() {
    let ddg = r#"<div><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fbevy.org%2Flearn%2F&amp;rut=x">Bevy <b>Learn</b></a>
    <a class="result__snippet" href="x">A refreshingly simple &amp; data-driven game engine</a></div>"#;
    let hits = crate::ai::web::parse_ddg(ddg);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].url, "https://bevy.org/learn/");
    assert_eq!(hits[0].title, "Bevy Learn");
    assert!(hits[0].snippet.contains("simple & data"));
    let bing = r#"<li class="b_algo"><h2><a href="https://www.rust-lang.org/" h="x">Rust Programming Language</a></h2><div><p>A language empowering everyone</p></div></li>"#;
    let b = crate::ai::web::parse_bing(bing);
    assert_eq!(b[0].url, "https://www.rust-lang.org/");
    let text = crate::ai::web::html_to_text("<html><head><title>x</title></head><body><script>var a=1;</script><h1>Hello</h1><p>World &#39;ok&#39;</p></body></html>");
    assert!(text.contains("Hello") && text.contains("World 'ok'") && !text.contains("var a"));
}

#[test]
fn memory_ranks_and_extracts() {
    use crate::ai::memory::{bm25, extract_facts};
    let docs = [
        "User likes neon cyberpunk levels",
        "Coins should spin",
        "Player jumps with space",
    ];
    let r = bm25(&docs, "cyberpunk neon level");
    assert_eq!(r[0].0, 0);
    let f = extract_facts("Merhaba. Oyunlarım hep gece temalı olsun! Bugün hava güzel.");
    assert_eq!(f, vec!["Oyunlarım hep gece temalı olsun".to_string()]);
}

#[test]
fn file_tools_are_sandboxed() {
    use crate::ai::tools::{execute_in, project_path};
    let dir = std::env::temp_dir().join(format!("forge-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(project_path(&dir, "../etc/passwd").is_err());
    assert!(project_path(&dir, "/etc/passwd").is_ok_and(|p| p.starts_with(&dir)));
    let mut s = Scene::starter();
    let call = |name: &str, args| ToolCall {
        name: name.into(),
        args,
    };
    execute_in(&mut s, &call("write_file", json!({"path": "Scripts/spin.rhai", "content": "fn update(dt) { this.ry += 1.0 * dt; }"})), Some(&dir)).unwrap();
    let ls = execute_in(&mut s, &call("list_files", json!({"path": ""})), Some(&dir))
        .unwrap()
        .result;
    assert!(ls.contains("Scripts/spin.rhai"), "{ls}");
    let g = execute_in(
        &mut s,
        &call("grep", json!({"pattern": "this.ry"})),
        Some(&dir),
    )
    .unwrap()
    .result;
    assert!(g.contains("spin.rhai:1"));
    execute_in(
        &mut s,
        &call(
            "set_script",
            json!({"target": "Cube", "file": "Scripts/spin.rhai"}),
        ),
        Some(&dir),
    )
    .unwrap();
    assert!(s
        .get(s.find("Cube").unwrap())
        .unwrap()
        .script
        .contains("this.ry"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prefab_roundtrip_and_reparent() {
    let mut s = Scene::starter();
    let cube = s.find("Cube").unwrap();
    let child = s.spawn("Child", MeshKind::Sphere);
    s.get_mut(child).unwrap().transform.position = glam::Vec3::new(2.0, 1.0, 0.0);
    s.set_parent(child, Some(cube));
    let w = s.world_matrix(child).w_axis.truncate();
    assert!((w - glam::Vec3::new(2.0, 1.0, 0.0)).length() < 1e-3, "{w}");
    let sub = s.subtree(cube);
    assert_eq!(sub.len(), 2);
    let n = s.entities.len();
    let root = s
        .instantiate(sub, Some(glam::Vec3::new(5.0, 0.0, 0.0)))
        .unwrap();
    assert_eq!(s.entities.len(), n + 2);
    assert_eq!(s.children(Some(root)).len(), 1);
}

#[test]
fn every_ui_glyph_exists_in_the_fonts() {
    // Any symbol used inside a string literal must be renderable (no tofu boxes in the UI).
    let ctx = egui::Context::default();
    let _ = ctx.run(Default::default(), |_| {});
    let mut used = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir("src")
        .unwrap()
        .chain(std::fs::read_dir("src/app").unwrap())
        .chain(std::fs::read_dir("src/ai").unwrap())
    {
        let p = entry.unwrap().path();
        if p.extension().map_or(true, |e| e != "rs") || p.ends_with("tests.rs") {
            continue;
        }
        for line in std::fs::read_to_string(&p).unwrap().lines() {
            let l = line.trim_start();
            if l.starts_with("//") || l.starts_with("//!") {
                continue;
            }
            for (i, part) in line.split('"').enumerate() {
                if i % 2 == 1 {
                    for c in part.chars().filter(|c| {
                        (*c as u32) >= 0x2000 && !(0x2500..0x2580).contains(&(*c as u32))
                    }) {
                        used.insert(c);
                    }
                }
            }
        }
    }
    let mut missing = vec![];
    ctx.fonts(|f| {
        for c in &used {
            if !f.has_glyphs(&egui::FontId::proportional(14.0), &c.to_string()) {
                missing.push(*c);
            }
        }
    });
    assert!(missing.is_empty(), "glyphs without a font: {missing:?}");
}

#[test]
fn store_parses_catalogs() {
    use crate::store::*;
    let ph = json!({
        "rock_wall_08": {"name": "Rock Wall 08", "type": 1, "tags": ["rock", "wall"], "download_count": 50},
        "wood_floor": {"name": "Wood Floor", "type": 1, "tags": ["wood"], "download_count": 90, "thumbnail_url": "https://cdn.example/wood.png"}
    });
    let items = parse_polyhaven(Kind::Textures, &ph).unwrap();
    assert_eq!(items[0].id, "wood_floor");
    assert!(items[1].thumb.contains("rock_wall_08"));
    assert!(matches(&items[1], "rock"));
    let files = json!({"Diffuse": {"1k": {"jpg": {"url": "https://dl.polyhaven.org/x_diff_1k.jpg"}}}, "nor_gl": {}});
    assert_eq!(
        ph_diffuse_url(&files).unwrap(),
        "https://dl.polyhaven.org/x_diff_1k.jpg"
    );
    let acg = json!({"foundAssets": [{"assetId": "Bricks076", "displayName": "Bricks 076", "previewImage": {"128-PNG": "a", "256-PNG": "https://acg/256.png"}, "tags": ["brick"]}]});
    let a = parse_ambientcg(&acg);
    assert_eq!(a[0].id, "Bricks076");
    assert_eq!(a[0].thumb, "https://acg/256.png");
}

#[test]
fn loads_and_normalizes_gltf() {
    // A 2×4×1 box corner triangle pair, embedded as base64.
    let pos: [f32; 9] = [0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 4.0, 1.0];
    let bytes: Vec<u8> = pos.iter().flat_map(|f| f.to_le_bytes()).collect();
    use base64_lite::encode;
    let gltf = json!({
        "asset": {"version": "2.0"},
        "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "translation": [10.0, 0.0, 0.0]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [2.0, 4.0, 1.0]}],
        "bufferViews": [{"buffer": 0, "byteLength": 36}],
        "buffers": [{"byteLength": 36, "uri": format!("data:application/octet-stream;base64,{}", encode(&bytes))}]
    });
    let p = std::env::temp_dir().join(format!("forge-tri-{}.gltf", std::process::id()));
    std::fs::write(&p, gltf.to_string()).unwrap();
    let m = crate::model::load(&p).unwrap();
    assert_eq!(m.verts.len(), 3 * crate::mesh::STRIDE);
    assert!(
        (m.extents - glam::Vec3::new(2.0, 4.0, 1.0)).length() < 1e-4,
        "{}",
        m.extents
    );
    for v in m.verts.chunks(crate::mesh::STRIDE) {
        assert!(v[0].abs() <= 0.5001 && v[1].abs() <= 0.5001 && v[2].abs() <= 0.5001);
    }
    let _ = std::fs::remove_file(p);
}

mod base64_lite {
    pub fn encode(b: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }
}

#[test]
fn local_hosts_bypass_proxy() {
    use crate::ai::client::bypass_proxy;
    assert!(bypass_proxy("http://127.0.0.1:8089/v1"));
    assert!(bypass_proxy("http://localhost:11434/v1"));
    assert!(bypass_proxy("http://[::1]:8420"));
    assert!(!bypass_proxy("https://openrouter.ai/api/v1"));
}

#[test]
fn audio_and_animation_assets_via_tools() {
    use crate::ai::tools::execute_in;
    let dir = std::env::temp_dir().join(format!("forge-av-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Scene::starter();
    let call = |name: &str, args| ToolCall {
        name: name.into(),
        args,
    };
    let r = execute_in(
        &mut s,
        &call(
            "generate_sound",
            json!({"preset": "coin", "name": "pickup"}),
        ),
        Some(&dir),
    )
    .unwrap()
    .result;
    assert!(r.contains("Audio/pickup.wav"), "{r}");
    let wav = std::fs::read(dir.join("Audio/pickup.wav")).unwrap();
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert!(crate::audio::duration(&dir.join("Audio/pickup.wav")).unwrap() > 0.05);

    execute_in(
        &mut s,
        &call(
            "create_animation",
            json!({"preset": "bob", "name": "float"}),
        ),
        Some(&dir),
    )
    .unwrap();
    let ls = execute_in(&mut s, &call("list_assets", json!({})), Some(&dir))
        .unwrap()
        .result;
    assert!(
        ls.contains("pickup.wav") && ls.contains(".anim.json"),
        "{ls}"
    );
    let info = crate::ai::tools::inspect_asset(&dir.join("Audio/pickup.wav")).unwrap();
    assert!(!info.is_empty());
    assert!(crate::ai::tools::is_mutation("generate_sound"));
    assert!(!crate::ai::tools::is_mutation("list_assets"));
    assert!(!crate::ai::tools::is_mutation("inspect_asset"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_plays_sounds_and_animations() {
    let mut scene = Scene::starter();
    let id = scene.find("Cube").unwrap();
    let start_y = scene.get(id).unwrap().transform.position.y;
    {
        let e = scene.get_mut(id).unwrap();
        e.animator = Some(crate::scene::Animator {
            clip: "preset:bob".into(),
            speed: 1.0,
            play_on_start: true,
        });
        e.script = "fn start() { play_sound(\"Audio/x.wav\", 0.5); }".into();
    }
    let (mut rt, _) = crate::runtime::Runtime::new(scene, None);
    let mut moved = false;
    for _ in 0..30 {
        rt.step(1.0 / 30.0);
        let y = rt.scene.get(id).unwrap().transform.position.y;
        if (y - start_y).abs() > 1e-3 {
            moved = true;
        }
    }
    assert!(moved, "animator did not move the entity");
    let sounds = rt.take_sounds();
    assert!(
        sounds
            .iter()
            .any(|(p, v, _)| p.ends_with("Audio/x.wav") && (*v - 0.5).abs() < 1e-3),
        "{sounds:?}"
    );
}
