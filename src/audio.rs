//! Audio: playback (rodio) + a built-in retro sound-effect generator (sfxr-style, CC0 output).

use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

pub const AUDIO_EXTS: [&str; 4] = ["wav", "ogg", "mp3", "flac"];

/// Lazily-opened audio output. Lives on the UI thread (the stream is not `Send`).
#[derive(Default)]
pub struct AudioEngine {
    out: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    failed: bool,
    sinks: Vec<rodio::Sink>,
    /// Looping sources keyed by entity id, so they can be stopped.
    loops: HashMap<u64, rodio::Sink>,
    pub master: f32,
    pub muted: bool,
}

impl AudioEngine {
    fn handle(&mut self) -> Option<&rodio::OutputStreamHandle> {
        if self.out.is_none() && !self.failed {
            match rodio::OutputStream::try_default() {
                Ok(o) => self.out = Some(o),
                Err(_) => self.failed = true,
            }
        }
        self.out.as_ref().map(|(_, h)| h)
    }

    /// Play a sound file once (or looped for `owner`). Returns an error string on failure.
    pub fn play(&mut self, path: &Path, volume: f32, looped: Option<u64>) -> Result<(), String> {
        if self.muted {
            return Ok(());
        }
        let master = if self.master <= 0.0 { 1.0 } else { self.master };
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let h = self.handle().ok_or("no audio output device")?.clone();
        let sink = rodio::Sink::try_new(&h).map_err(|e| e.to_string())?;
        let src = rodio::Decoder::new(BufReader::new(file))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        sink.set_volume((volume * master).clamp(0.0, 4.0));
        match looped {
            Some(id) => {
                use rodio::Source;
                sink.append(src.repeat_infinite());
                if let Some(old) = self.loops.insert(id, sink) {
                    old.stop();
                }
            }
            None => {
                sink.append(src);
                self.sinks.retain(|s| !s.empty());
                self.sinks.push(sink);
            }
        }
        Ok(())
    }

    pub fn stop_all(&mut self) {
        for s in self.sinks.drain(..) {
            s.stop();
        }
        for (_, s) in self.loops.drain() {
            s.stop();
        }
    }
}

// ------------------------------------------------------------ SFX generator

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    Coin,
    Jump,
    Laser,
    Explosion,
    Hit,
    PowerUp,
    Click,
    Blip,
}

impl Preset {
    pub const ALL: [Preset; 8] = [
        Preset::Coin,
        Preset::Jump,
        Preset::Laser,
        Preset::Explosion,
        Preset::Hit,
        Preset::PowerUp,
        Preset::Click,
        Preset::Blip,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Preset::Coin => "coin",
            Preset::Jump => "jump",
            Preset::Laser => "laser",
            Preset::Explosion => "explosion",
            Preset::Hit => "hit",
            Preset::PowerUp => "powerup",
            Preset::Click => "click",
            Preset::Blip => "blip",
        }
    }
    pub fn parse(s: &str) -> Option<Preset> {
        let s = s.to_lowercase();
        Preset::ALL
            .into_iter()
            .find(|p| s.contains(p.name()) || (p.name() == "powerup" && s.contains("power")))
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Square,
    Saw,
    Sine,
    Noise,
}

struct Params {
    wave: Wave,
    freq: f32,
    slide: f32,
    attack: f32,
    sustain: f32,
    decay: f32,
    arp_at: f32,
    arp_mul: f32,
    vibrato: f32,
}

fn params(p: Preset, rnd: &mut impl FnMut() -> f32) -> Params {
    let r = |rnd: &mut dyn FnMut() -> f32, a: f32, b: f32| a + rnd() * (b - a);
    match p {
        Preset::Coin => Params {
            wave: Wave::Square,
            freq: r(rnd, 700.0, 1100.0),
            slide: 0.0,
            attack: 0.0,
            sustain: 0.05,
            decay: r(rnd, 0.15, 0.3),
            arp_at: 0.06,
            arp_mul: r(rnd, 1.3, 1.6),
            vibrato: 0.0,
        },
        Preset::Jump => Params {
            wave: Wave::Square,
            freq: r(rnd, 250.0, 400.0),
            slide: r(rnd, 1.5, 3.0),
            attack: 0.0,
            sustain: 0.08,
            decay: 0.18,
            arp_at: 1.0,
            arp_mul: 1.0,
            vibrato: 0.0,
        },
        Preset::Laser => Params {
            wave: Wave::Saw,
            freq: r(rnd, 900.0, 1500.0),
            slide: r(rnd, -3.5, -2.0),
            attack: 0.0,
            sustain: 0.05,
            decay: 0.15,
            arp_at: 1.0,
            arp_mul: 1.0,
            vibrato: 0.0,
        },
        Preset::Explosion => Params {
            wave: Wave::Noise,
            freq: r(rnd, 80.0, 200.0),
            slide: -0.4,
            attack: 0.0,
            sustain: 0.12,
            decay: r(rnd, 0.4, 0.7),
            arp_at: 1.0,
            arp_mul: 1.0,
            vibrato: 0.0,
        },
        Preset::Hit => Params {
            wave: Wave::Noise,
            freq: r(rnd, 300.0, 600.0),
            slide: -1.5,
            attack: 0.0,
            sustain: 0.02,
            decay: 0.12,
            arp_at: 1.0,
            arp_mul: 1.0,
            vibrato: 0.0,
        },
        Preset::PowerUp => Params {
            wave: Wave::Square,
            freq: r(rnd, 300.0, 450.0),
            slide: 1.2,
            attack: 0.0,
            sustain: 0.2,
            decay: 0.25,
            arp_at: 1.0,
            arp_mul: 1.0,
            vibrato: r(rnd, 6.0, 12.0),
        },
        Preset::Click => Params {
            wave: Wave::Square,
            freq: r(rnd, 1200.0, 1800.0),
            slide: 0.0,
            attack: 0.0,
            sustain: 0.005,
            decay: 0.03,
            arp_at: 1.0,
            arp_mul: 1.0,
            vibrato: 0.0,
        },
        Preset::Blip => Params {
            wave: Wave::Sine,
            freq: r(rnd, 500.0, 900.0),
            slide: 0.0,
            attack: 0.005,
            sustain: 0.04,
            decay: 0.08,
            arp_at: 1.0,
            arp_mul: 1.0,
            vibrato: 0.0,
        },
    }
}

/// Synthesize a preset as 16-bit mono PCM samples at 44.1 kHz.
pub fn synth(p: Preset, seed: u64) -> Vec<i16> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
        | 1;
    let mut rnd = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    let pr = params(p, &mut rnd);
    let sr = 44_100.0;
    let total = pr.attack + pr.sustain + pr.decay;
    let n = (total * sr) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    let mut noise = 0.0f32;
    for i in 0..n {
        let t = i as f32 / sr;
        let env = if t < pr.attack {
            t / pr.attack.max(1e-4)
        } else if t < pr.attack + pr.sustain {
            1.0
        } else {
            (1.0 - (t - pr.attack - pr.sustain) / pr.decay.max(1e-4))
                .max(0.0)
                .powf(1.6)
        };
        let mut f = pr.freq * (pr.slide * t).exp2();
        if t >= pr.arp_at {
            f *= pr.arp_mul;
        }
        if pr.vibrato > 0.0 {
            f *= 1.0 + (t * pr.vibrato * std::f32::consts::TAU).sin() * 0.04;
        }
        let prev = phase;
        phase = (phase + f / sr).fract();
        let s = match pr.wave {
            Wave::Square => {
                if phase < 0.5 {
                    0.6
                } else {
                    -0.6
                }
            }
            Wave::Saw => phase * 1.2 - 0.6,
            Wave::Sine => (phase * std::f32::consts::TAU).sin() * 0.8,
            Wave::Noise => {
                if phase < prev {
                    noise = rnd() * 2.0 - 1.0;
                }
                noise * 0.7
            }
        };
        out.push((s * env * 0.8 * i16::MAX as f32) as i16);
    }
    out
}

pub fn write_wav(path: &Path, samples: &[i16]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&44_100u32.to_le_bytes());
    b.extend_from_slice(&(44_100u32 * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b)
}

/// Generate a preset into `<root>/Audio/<name>.wav`; returns the project-relative path.
pub fn generate(root: &Path, preset: Preset, name: &str, seed: u64) -> Result<String, String> {
    let safe: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let safe = if safe.is_empty() {
        preset.name().to_string()
    } else {
        safe
    };
    let rel = format!("Audio/{safe}.wav");
    write_wav(&root.join(&rel), &synth(preset, seed)).map_err(|e| e.to_string())?;
    Ok(rel)
}

/// Length of an audio file in seconds (when the decoder knows it).
pub fn duration(p: &Path) -> Option<f32> {
    use rodio::Source;
    let f = std::fs::File::open(p).ok()?;
    rodio::Decoder::new(BufReader::new(f))
        .ok()?
        .total_duration()
        .map(|d| d.as_secs_f32())
}

pub fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map_or(false, |e| AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
}
