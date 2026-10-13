//! Project templates.

use crate::scene::{CameraComp, MeshKind, Rigidbody, Scene};
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Empty,
    Starter,
    Platformer,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::Starter, Template::Platformer, Template::Empty];
    pub fn label(self) -> &'static str {
        match self {
            Template::Empty => "Empty",
            Template::Starter => "3D Starter (ground, light, camera)",
            Template::Platformer => "Platformer sample (playable)",
        }
    }
    pub fn build(self) -> Scene {
        match self {
            Template::Empty => Scene { name: "Main".into(), ..Default::default() },
            Template::Starter => Scene::starter(),
            Template::Platformer => platformer(),
        }
    }
}

const PLAYER: &str = r#"fn start() { this.speed = 7.0; this.jump = 9.0; set_global("score", 0); }
fn update(dt) {
    if key("A") || key("LEFT")  { this.x -= this.speed * dt; }
    if key("D") || key("RIGHT") { this.x += this.speed * dt; }
    if key("W") || key("UP")    { this.z -= this.speed * dt; }
    if key("S") || key("DOWN")  { this.z += this.speed * dt; }
    if key_pressed("SPACE") && this.grounded { this.vy = this.jump; }
    let n = destroy_touching(this.name, "Coin");
    if n > 0 { set_global("score", get_global("score") + n); }
    let s = get_global("score");
    if s >= 5 { hud("You win! Score: " + s); } else { hud("Coins: " + s + " / 5   (WASD + Space)"); }
    if this.y < -10.0 { this.x = 0.0; this.y = 2.0; this.z = 0.0; this.vy = 0.0; }
}
"#;

const COIN: &str = r#"fn update(dt) { this.ry += 120.0 * dt; this.y = 1.2 + (this.time * 3.0).sin() * 0.15; }"#;

fn platformer() -> Scene {
    let mut s = Scene::starter();
    s.name = "Platformer".into();
    if let Some(c) = s.find("Cube") {
        s.remove(c);
    }
    let p = s.spawn("Player", MeshKind::Capsule);
    {
        let e = s.get_mut(p).unwrap();
        e.transform.position = Vec3::new(0.0, 1.0, 0.0);
        e.transform.scale = Vec3::new(1.0, 1.6, 1.0);
        e.color = [0.94, 0.35, 0.12];
        e.rigidbody = Some(Rigidbody::default());
        e.script = PLAYER.into();
    }
    let plats = [(4.0, 1.0, -3.0), (8.0, 2.0, -6.0), (3.0, 3.0, -10.0), (-3.0, 2.0, -7.0), (-7.0, 1.0, -3.0)];
    for (i, (x, y, z)) in plats.iter().enumerate() {
        let id = s.spawn(&format!("Platform {}", i + 1), MeshKind::Cube);
        let e = s.get_mut(id).unwrap();
        e.transform.position = Vec3::new(*x, *y - 0.25, *z);
        e.transform.scale = Vec3::new(3.0, 0.5, 3.0);
        e.color = [0.89, 0.88, 0.86];
        let c = s.spawn(&format!("Coin {}", i + 1), MeshKind::Cylinder);
        let e = s.get_mut(c).unwrap();
        e.transform.position = Vec3::new(*x, *y + 1.2, *z);
        e.transform.scale = Vec3::new(0.6, 0.12, 0.6);
        e.transform.rotation = Vec3::new(90.0, 0.0, 0.0);
        e.color = [0.96, 0.66, 0.13];
        e.trigger = true;
        e.tag = "Coin".into();
        e.script = COIN.into();
    }
    if let Some(cam) = s.find("Main Camera") {
        let e = s.get_mut(cam).unwrap();
        e.transform.position = Vec3::new(0.0, 7.0, 11.0);
        e.transform.rotation = Vec3::new(-25.0, 0.0, 0.0);
        e.camera = Some(CameraComp { fov: 60.0, follow: "Player".into() });
    }
    s
}
