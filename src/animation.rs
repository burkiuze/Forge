//! Keyframe animation clips (`.anim.json`) applied as offsets on top of an entity's transform.
//!
//! Tracks animate `position` (offset), `rotation` (offset, degrees), `scale` (multiplier) and
//! `color` (absolute). Keys are linearly interpolated; clips can loop.

use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Property {
    Position,
    Rotation,
    Scale,
    Color,
}

impl Property {
    pub const ALL: [Property; 4] = [
        Property::Position,
        Property::Rotation,
        Property::Scale,
        Property::Color,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Property::Position => "Position",
            Property::Rotation => "Rotation",
            Property::Scale => "Scale",
            Property::Color => "Color",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Key {
    pub t: f32,
    pub v: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub property: Property,
    pub keys: Vec<Key>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Clip {
    pub name: String,
    pub length: f32,
    pub looping: bool,
    pub tracks: Vec<Track>,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            name: "New Clip".into(),
            length: 1.0,
            looping: true,
            tracks: vec![],
        }
    }
}

/// Result of sampling a clip.
#[derive(Default, Clone, Copy)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
    pub color: Option<[f32; 3]>,
}

impl Clip {
    pub fn load(path: &Path) -> Result<Clip, String> {
        serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap()).map_err(|e| e.to_string())
    }

    pub fn track_mut(&mut self, p: Property) -> &mut Track {
        if let Some(i) = self.tracks.iter().position(|t| t.property == p) {
            return &mut self.tracks[i];
        }
        self.tracks.push(Track {
            property: p,
            keys: vec![],
        });
        self.tracks.last_mut().unwrap()
    }

    /// Insert or replace a key at time `t`.
    pub fn set_key(&mut self, p: Property, t: f32, v: [f32; 3]) {
        let len = self.length;
        let tr = self.track_mut(p);
        let t = t.clamp(0.0, len);
        match tr.keys.iter_mut().find(|k| (k.t - t).abs() < 1e-3) {
            Some(k) => k.v = v,
            None => tr.keys.push(Key { t, v }),
        }
        tr.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    }

    pub fn sample(&self, time: f32) -> Pose {
        let t = if self.looping && self.length > 0.0 {
            time.rem_euclid(self.length)
        } else {
            time.min(self.length)
        };
        let mut pose = Pose {
            scale: Vec3::ONE,
            ..Default::default()
        };
        for tr in &self.tracks {
            let Some(v) = sample_track(&tr.keys, t) else {
                continue;
            };
            match tr.property {
                Property::Position => pose.position = Vec3::from(v),
                Property::Rotation => pose.rotation = Vec3::from(v),
                Property::Scale => pose.scale = Vec3::from(v),
                Property::Color => pose.color = Some(v),
            }
        }
        pose
    }

    pub fn finished(&self, time: f32) -> bool {
        !self.looping && time >= self.length
    }
}

fn sample_track(keys: &[Key], t: f32) -> Option<[f32; 3]> {
    let first = keys.first()?;
    if t <= first.t {
        return Some(first.v);
    }
    for w in keys.windows(2) {
        if t <= w[1].t {
            let f = (t - w[0].t) / (w[1].t - w[0].t).max(1e-5);
            let lerp = |i: usize| w[0].v[i] + (w[1].v[i] - w[0].v[i]) * f;
            return Some([lerp(0), lerp(1), lerp(2)]);
        }
    }
    keys.last().map(|k| k.v)
}

/// Ready-made clips (also offered to the agent through `create_animation {preset}`).
pub fn preset(name: &str) -> Option<Clip> {
    let k = |t: f32, v: [f32; 3]| Key { t, v };
    let clip = |name: &str, length: f32, looping: bool, tracks: Vec<Track>| Clip {
        name: name.into(),
        length,
        looping,
        tracks,
    };
    let tr = |property, keys| Track { property, keys };
    Some(match name.to_lowercase().as_str() {
        "bob" | "idle" | "float" => clip(
            "Bob",
            2.0,
            true,
            vec![tr(
                Property::Position,
                vec![k(0.0, [0.0; 3]), k(1.0, [0.0, 0.25, 0.0]), k(2.0, [0.0; 3])],
            )],
        ),
        "spin" | "rotate" => clip(
            "Spin",
            2.0,
            true,
            vec![tr(
                Property::Rotation,
                vec![k(0.0, [0.0; 3]), k(2.0, [0.0, 360.0, 0.0])],
            )],
        ),
        "pulse" | "breathe" => clip(
            "Pulse",
            1.0,
            true,
            vec![tr(
                Property::Scale,
                vec![k(0.0, [1.0; 3]), k(0.5, [1.15; 3]), k(1.0, [1.0; 3])],
            )],
        ),
        "walk" | "walk_bounce" => clip(
            "Walk",
            0.5,
            true,
            vec![
                tr(
                    Property::Position,
                    vec![
                        k(0.0, [0.0; 3]),
                        k(0.125, [0.0, 0.08, 0.0]),
                        k(0.25, [0.0; 3]),
                        k(0.375, [0.0, 0.08, 0.0]),
                        k(0.5, [0.0; 3]),
                    ],
                ),
                tr(
                    Property::Rotation,
                    vec![
                        k(0.0, [0.0, 0.0, -4.0]),
                        k(0.25, [0.0, 0.0, 4.0]),
                        k(0.5, [0.0, 0.0, -4.0]),
                    ],
                ),
            ],
        ),
        "swing" | "arm_swing" | "leg_swing" => clip(
            "Swing",
            0.6,
            true,
            vec![tr(
                Property::Rotation,
                vec![
                    k(0.0, [30.0, 0.0, 0.0]),
                    k(0.3, [-30.0, 0.0, 0.0]),
                    k(0.6, [30.0, 0.0, 0.0]),
                ],
            )],
        ),
        "swing_alt" | "swing2" => clip(
            "Swing Alt",
            0.6,
            true,
            vec![tr(
                Property::Rotation,
                vec![
                    k(0.0, [-30.0, 0.0, 0.0]),
                    k(0.3, [30.0, 0.0, 0.0]),
                    k(0.6, [-30.0, 0.0, 0.0]),
                ],
            )],
        ),
        "door" | "door_open" => clip(
            "Door Open",
            1.0,
            false,
            vec![tr(
                Property::Rotation,
                vec![k(0.0, [0.0; 3]), k(1.0, [0.0, -100.0, 0.0])],
            )],
        ),
        "shake" => clip(
            "Shake",
            0.4,
            true,
            vec![tr(
                Property::Position,
                vec![
                    k(0.0, [0.0; 3]),
                    k(0.1, [0.08, 0.0, 0.0]),
                    k(0.2, [-0.08, 0.0, 0.0]),
                    k(0.3, [0.05, 0.0, 0.0]),
                    k(0.4, [0.0; 3]),
                ],
            )],
        ),
        "blink" | "flash" => clip(
            "Flash",
            0.6,
            true,
            vec![tr(
                Property::Color,
                vec![
                    k(0.0, [1.0, 1.0, 1.0]),
                    k(0.3, [1.0, 0.3, 0.2]),
                    k(0.6, [1.0, 1.0, 1.0]),
                ],
            )],
        ),
        "hover" => clip(
            "Hover",
            3.0,
            true,
            vec![tr(
                Property::Position,
                vec![
                    k(0.0, [0.0; 3]),
                    k(0.75, [0.3, 0.2, 0.0]),
                    k(1.5, [0.0, 0.4, 0.0]),
                    k(2.25, [-0.3, 0.2, 0.0]),
                    k(3.0, [0.0; 3]),
                ],
            )],
        ),
        _ => return None,
    })
}

pub const PRESETS: [&str; 10] = [
    "bob",
    "spin",
    "pulse",
    "walk",
    "swing",
    "swing_alt",
    "door",
    "shake",
    "flash",
    "hover",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_and_loops() {
        let c = preset("bob").unwrap();
        assert!((c.sample(1.0).position.y - 0.25).abs() < 1e-4);
        assert!((c.sample(0.5).position.y - 0.125).abs() < 1e-4);
        assert!((c.sample(3.0).position.y - 0.25).abs() < 1e-4, "loops");
        let d = preset("door").unwrap();
        assert!((d.sample(5.0).rotation.y + 100.0).abs() < 1e-3);
        assert!(d.finished(1.5));
        let mut n = Clip::default();
        n.set_key(Property::Scale, 0.5, [2.0; 3]);
        n.set_key(Property::Scale, 0.5, [3.0; 3]);
        assert_eq!(n.tracks[0].keys.len(), 1);
        assert_eq!(n.sample(0.7).scale, Vec3::splat(3.0));
    }
}
