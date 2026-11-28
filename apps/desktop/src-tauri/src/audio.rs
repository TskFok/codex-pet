//! 桌宠程序化音效合成与原生异步播放模块。
//!
//! 为桌面互动、自由活动微音效及分级任务提醒提供零外部音频资源依赖的实时 WAV 合成，
//! 支持多种音色主题（治愈萌音、复古像素、极简自然）与按宠物身份自动微调的专属声线。

use crate::settings::{DEEPSEEK_WHALE_MAID, SoundPack};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    f32::consts::TAU,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const SAMPLE_RATE: u32 = 44_100;
const MAX_WAV_CACHE_ENTRIES: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundCategory {
    Interaction,
    Ambient,
    Alert,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PetSoundCue {
    PetHead,
    PokeBody,
    Dodge,
    TickleFeet,
    DoubleClick,
    TripleCombo,
    Dizzy,
    DizzyRecover,
    Feed,
    Play,
    Showcase,
    DragDrop,
    AmbientHop,
    AmbientWave,
    AmbientMurmur,
    AlertApproval,
    AlertAnswer,
    AlertComplete,
    AlertFailed,
    AlertInterrupted,
}

impl PetSoundCue {
    pub const ALL: &'static [Self] = &[
        Self::PetHead,
        Self::PokeBody,
        Self::Dodge,
        Self::TickleFeet,
        Self::DoubleClick,
        Self::TripleCombo,
        Self::Dizzy,
        Self::DizzyRecover,
        Self::Feed,
        Self::Play,
        Self::Showcase,
        Self::DragDrop,
        Self::AmbientHop,
        Self::AmbientWave,
        Self::AmbientMurmur,
        Self::AlertApproval,
        Self::AlertAnswer,
        Self::AlertComplete,
        Self::AlertFailed,
        Self::AlertInterrupted,
    ];

    pub fn category(self) -> SoundCategory {
        match self {
            Self::PetHead
            | Self::PokeBody
            | Self::Dodge
            | Self::TickleFeet
            | Self::DoubleClick
            | Self::TripleCombo
            | Self::Dizzy
            | Self::DizzyRecover
            | Self::Feed
            | Self::Play
            | Self::Showcase
            | Self::DragDrop => SoundCategory::Interaction,
            Self::AmbientHop | Self::AmbientWave | Self::AmbientMurmur => SoundCategory::Ambient,
            Self::AlertApproval
            | Self::AlertAnswer
            | Self::AlertComplete
            | Self::AlertFailed
            | Self::AlertInterrupted => SoundCategory::Alert,
        }
    }

    pub fn cooldown(self) -> Duration {
        match self {
            Self::Dizzy | Self::TripleCombo | Self::DoubleClick | Self::Feed | Self::Play => {
                Duration::from_millis(35)
            }
            Self::PetHead
            | Self::PokeBody
            | Self::Dodge
            | Self::TickleFeet
            | Self::DizzyRecover
            | Self::Showcase
            | Self::DragDrop => Duration::from_millis(65),
            Self::AmbientHop | Self::AmbientWave | Self::AmbientMurmur => {
                Duration::from_millis(3_500)
            }
            Self::AlertApproval
            | Self::AlertAnswer
            | Self::AlertComplete
            | Self::AlertFailed
            | Self::AlertInterrupted => Duration::from_millis(120),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceProfile {
    pub pitch_mul: f32,
    pub tempo_mul: f32,
    pub brightness: f32,
}

pub fn voice_profile_for_pet(pet_id: &str) -> VoiceProfile {
    match pet_id {
        "yueying-cape-dancer" => VoiceProfile {
            pitch_mul: 1.05,
            tempo_mul: 1.0,
            brightness: 1.12,
        },
        "deepseek-big-fish" => VoiceProfile {
            pitch_mul: 0.89,
            tempo_mul: 1.06,
            brightness: 0.86,
        },
        DEEPSEEK_WHALE_MAID => VoiceProfile {
            pitch_mul: 1.16,
            tempo_mul: 1.14,
            brightness: 1.18,
        },
        "zhizhi-branch-guardian" => VoiceProfile {
            pitch_mul: 1.12,
            tempo_mul: 0.93,
            brightness: 1.08,
        },
        other => {
            let mut hash: u32 = 2_166_136_261;
            for byte in other.bytes() {
                hash ^= u32::from(byte);
                hash = hash.wrapping_mul(16_777_619);
            }
            let norm = (hash % 1000) as f32 / 999.0;
            let bright_seed = ((hash / 1000) % 100) as f32 / 500.0;
            VoiceProfile {
                pitch_mul: 0.92 + norm * 0.18,
                tempo_mul: 0.96 + (1.0 - norm) * 0.08,
                brightness: 0.92 + bright_seed,
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Role {
    Chirp,
    Bell,
    Sparkle,
    SoftThud,
}

#[derive(Clone, Copy)]
struct NoteSpec {
    start_sec: f32,
    duration_sec: f32,
    freq_start_hz: f32,
    freq_end_hz: f32,
    pitch_curve: f32,
    vibrato_hz: f32,
    vibrato_depth: f32,
    attack_sec: f32,
    decay_rate: f32,
    gain: f32,
    role: Role,
}

impl NoteSpec {
    const fn chirp(start: f32, dur: f32, f0: f32, f1: f32, gain: f32) -> Self {
        Self {
            start_sec: start,
            duration_sec: dur,
            freq_start_hz: f0,
            freq_end_hz: f1,
            pitch_curve: 0.72,
            vibrato_hz: 5.5,
            vibrato_depth: 0.006,
            attack_sec: 0.006,
            decay_rate: 6.5,
            gain,
            role: Role::Chirp,
        }
    }

    const fn bell(start: f32, dur: f32, freq: f32, gain: f32) -> Self {
        Self {
            start_sec: start,
            duration_sec: dur,
            freq_start_hz: freq,
            freq_end_hz: freq,
            pitch_curve: 1.0,
            vibrato_hz: 4.5,
            vibrato_depth: 0.003,
            attack_sec: 0.005,
            decay_rate: 7.5,
            gain,
            role: Role::Bell,
        }
    }

    const fn sparkle(start: f32, dur: f32, f0: f32, f1: f32, gain: f32) -> Self {
        Self {
            start_sec: start,
            duration_sec: dur,
            freq_start_hz: f0,
            freq_end_hz: f1,
            pitch_curve: 0.85,
            vibrato_hz: 6.5,
            vibrato_depth: 0.005,
            attack_sec: 0.004,
            decay_rate: 9.0,
            gain,
            role: Role::Sparkle,
        }
    }

    const fn wobble(
        start: f32,
        dur: f32,
        f0: f32,
        f1: f32,
        vib_hz: f32,
        depth: f32,
        gain: f32,
    ) -> Self {
        Self {
            start_sec: start,
            duration_sec: dur,
            freq_start_hz: f0,
            freq_end_hz: f1,
            pitch_curve: 1.0,
            vibrato_hz: vib_hz,
            vibrato_depth: depth,
            attack_sec: 0.006,
            decay_rate: 5.2,
            gain,
            role: Role::Chirp,
        }
    }

    const fn thud(start: f32, dur: f32, f0: f32, f1: f32, gain: f32) -> Self {
        Self {
            start_sec: start,
            duration_sec: dur,
            freq_start_hz: f0,
            freq_end_hz: f1,
            pitch_curve: 0.6,
            vibrato_hz: 0.0,
            vibrato_depth: 0.0,
            attack_sec: 0.004,
            decay_rate: 14.0,
            gain,
            role: Role::SoftThud,
        }
    }
}

const G4: f32 = 392.00;
const A4: f32 = 440.00;
const B4: f32 = 493.88;
const C5: f32 = 523.25;
const D5: f32 = 587.33;
const EB5: f32 = 622.25;
const E5: f32 = 659.25;
const F5: f32 = 698.46;
const FS5: f32 = 739.99;
const G5: f32 = 783.99;
const A5: f32 = 880.00;
const B5: f32 = 987.77;
const C6: f32 = 1046.50;
const CS6: f32 = 1108.73;
const D6: f32 = 1174.66;
const E6: f32 = 1318.51;
const F6: f32 = 1396.91;
const FS6: f32 = 1479.98;
const G6: f32 = 1567.98;

fn cue_score(cue: PetSoundCue) -> &'static [NoteSpec] {
    match cue {
        PetSoundCue::PetHead => {
            const {
                &[
                    NoteSpec::chirp(0.00, 0.13, E5, G5, 0.52),
                    NoteSpec::chirp(0.085, 0.17, G5, B5, 0.56),
                    NoteSpec::sparkle(0.11, 0.15, E6, E6, 0.20),
                ]
            }
        }
        PetSoundCue::PokeBody => {
            const {
                &[
                    NoteSpec::chirp(0.00, 0.11, G5, C6, 0.54),
                    NoteSpec::sparkle(0.035, 0.09, E6, E6, 0.18),
                ]
            }
        }
        PetSoundCue::Dodge => {
            const {
                &[
                    NoteSpec::chirp(0.00, 0.085, D5, A5, 0.52),
                    NoteSpec::chirp(0.07, 0.11, G5, E5, 0.46),
                ]
            }
        }
        PetSoundCue::TickleFeet => {
            const {
                &[
                    NoteSpec::chirp(0.00, 0.065, E5, FS5, 0.46),
                    NoteSpec::chirp(0.055, 0.065, G5, A5, 0.48),
                    NoteSpec::chirp(0.11, 0.07, C6, D6, 0.50),
                    NoteSpec::sparkle(0.165, 0.11, E6, G6, 0.44),
                ]
            }
        }
        PetSoundCue::DoubleClick => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.11, C5, 0.50),
                    NoteSpec::bell(0.065, 0.15, G5, 0.54),
                    NoteSpec::sparkle(0.075, 0.14, D6, D6, 0.24),
                ]
            }
        }
        PetSoundCue::TripleCombo => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.085, C5, 0.48),
                    NoteSpec::bell(0.06, 0.085, E5, 0.50),
                    NoteSpec::bell(0.12, 0.095, G5, 0.52),
                    NoteSpec::bell(0.18, 0.22, C6, 0.56),
                    NoteSpec::sparkle(0.19, 0.20, E6, G6, 0.30),
                ]
            }
        }
        PetSoundCue::Dizzy => {
            const {
                &[
                    NoteSpec::wobble(0.00, 0.16, A5, F5, 10.0, 0.042, 0.50),
                    NoteSpec::wobble(0.135, 0.16, FS5, D5, 9.5, 0.045, 0.48),
                    NoteSpec::wobble(0.27, 0.22, EB5, B4, 8.5, 0.048, 0.46),
                ]
            }
        }
        PetSoundCue::DizzyRecover => {
            const { &[NoteSpec::wobble(0.00, 0.17, D5, B4, 7.5, 0.025, 0.36)] }
        }
        PetSoundCue::Feed => {
            const {
                &[
                    NoteSpec::thud(0.00, 0.065, F5, G5, 0.44),
                    NoteSpec::thud(0.08, 0.065, G5, A5, 0.46),
                    NoteSpec::bell(0.17, 0.12, A5, 0.48),
                    NoteSpec::bell(0.24, 0.19, C6, 0.54),
                    NoteSpec::sparkle(0.25, 0.17, F6, F6, 0.26),
                ]
            }
        }
        PetSoundCue::Play => {
            const {
                &[
                    NoteSpec::chirp(0.00, 0.075, G5, A5, 0.48),
                    NoteSpec::chirp(0.07, 0.075, A5, B5, 0.50),
                    NoteSpec::bell(0.145, 0.085, C6, 0.52),
                    NoteSpec::chirp(0.225, 0.16, G5, E6, 0.54),
                ]
            }
        }
        PetSoundCue::Showcase => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.085, D5, 0.46),
                    NoteSpec::bell(0.06, 0.095, A5, 0.50),
                    NoteSpec::sparkle(0.12, 0.17, D6, FS6, 0.46),
                ]
            }
        }
        PetSoundCue::DragDrop => {
            const {
                &[
                    NoteSpec::thud(0.00, 0.08, G4, 293.66, 0.46),
                    NoteSpec::chirp(0.06, 0.15, D5, G5, 0.48),
                ]
            }
        }
        PetSoundCue::AmbientHop => const { &[NoteSpec::chirp(0.00, 0.10, G5, B5, 0.28)] },
        PetSoundCue::AmbientWave => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.075, E5, 0.24),
                    NoteSpec::bell(0.055, 0.10, A5, 0.26),
                ]
            }
        }
        PetSoundCue::AmbientMurmur => const { &[NoteSpec::chirp(0.00, 0.09, D5, FS5, 0.24)] },
        PetSoundCue::AlertApproval => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.10, A5, 0.52),
                    NoteSpec::bell(0.085, 0.095, E5, 0.46),
                    NoteSpec::bell(0.175, 0.20, A5, 0.56),
                    NoteSpec::sparkle(0.18, 0.19, CS6, CS6, 0.28),
                ]
            }
        }
        PetSoundCue::AlertAnswer => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.09, F5, 0.48),
                    NoteSpec::bell(0.08, 0.09, A5, 0.50),
                    NoteSpec::chirp(0.16, 0.17, C6, D6, 0.56),
                ]
            }
        }
        PetSoundCue::AlertComplete => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.09, C5, 0.46),
                    NoteSpec::bell(0.06, 0.095, E5, 0.48),
                    NoteSpec::bell(0.12, 0.11, G5, 0.50),
                    NoteSpec::bell(0.18, 0.23, B5, 0.52),
                    NoteSpec::sparkle(0.185, 0.21, E6, E6, 0.26),
                ]
            }
        }
        PetSoundCue::AlertFailed => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.12, E5, 0.50),
                    NoteSpec::bell(0.105, 0.12, C5, 0.48),
                    NoteSpec::bell(0.21, 0.18, A4, 0.52),
                ]
            }
        }
        PetSoundCue::AlertInterrupted => {
            const {
                &[
                    NoteSpec::bell(0.00, 0.09, G5, 0.46),
                    NoteSpec::bell(0.095, 0.13, D5, 0.46),
                ]
            }
        }
    }
}

#[inline]
fn poly_blep(phase: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if phase < dt {
        let u = phase / dt;
        u + u - u * u - 1.0
    } else if phase > 1.0 - dt {
        let u = (phase - 1.0) / dt;
        u * u + u + u + 1.0
    } else {
        0.0
    }
}

#[inline]
fn pulse_blep(phase: f32, dt: f32, duty: f32) -> f32 {
    let raw = if phase < duty { 1.0 } else { -1.0 };
    let shifted = (phase + 1.0 - duty).fract();
    (raw + poly_blep(phase, dt) - poly_blep(shifted, dt)) * 0.62
}

#[inline]
fn triangle_8bit(phase: f32) -> f32 {
    let tri = if phase < 0.5 {
        4.0 * phase - 1.0
    } else {
        3.0 - 4.0 * phase
    };
    (tri * 7.5).round() / 7.5
}

#[inline]
fn soft_limit(sample: f32) -> f32 {
    let x = sample * 0.92;
    x / (1.0 + x.abs() * 0.38)
}

fn sample_waveform(
    pack: SoundPack,
    role: Role,
    phase: f32,
    dt: f32,
    local_t: f32,
    brightness: f32,
) -> f32 {
    let omega = phase * TAU;
    match pack {
        SoundPack::Healing => match role {
            Role::Chirp => {
                omega.sin() + 0.16 * brightness * (2.0 * omega).sin() * (-4.0 * local_t).exp()
            }
            Role::Bell => {
                let overtone_env = (-6.5 * local_t).exp();
                omega.sin()
                    + 0.24 * brightness * (2.0 * omega).sin() * overtone_env
                    + 0.07 * brightness * (3.0 * omega).sin() * overtone_env * overtone_env
            }
            Role::Sparkle => {
                let shimmer = (-9.0 * local_t).exp();
                omega.sin() * 0.85
                    + 0.28 * brightness * (2.0 * omega).sin() * shimmer
                    + 0.12 * brightness * (4.0 * omega).sin() * shimmer
            }
            Role::SoftThud => omega.sin() + 0.22 * (1.5 * omega).sin() * (-12.0 * local_t).exp(),
        },
        SoundPack::Retro => match role {
            Role::Chirp => pulse_blep(phase, dt, 0.25),
            Role::Bell => pulse_blep(phase, dt, 0.50) * 0.82 + triangle_8bit(phase) * 0.28,
            Role::Sparkle => pulse_blep(phase, dt, 0.125) * 0.9,
            Role::SoftThud => triangle_8bit(phase),
        },
        SoundPack::Minimal => match role {
            Role::Chirp | Role::SoftThud => {
                // 水滴/木琴模态泛音：基频 + 快速衰减的非谐木质泛音
                let modal = (-22.0 * local_t).exp();
                omega.sin() + 0.26 * brightness * (2.76 * omega).sin() * modal
            }
            Role::Bell | Role::Sparkle => {
                // 风铃/拇指琴：清透基频与轻柔高次泛音
                let chime = (-12.0 * local_t).exp();
                omega.sin()
                    + 0.18 * brightness * (2.0 * omega).sin() * chime
                    + 0.08 * brightness * (3.01 * omega).sin() * chime
            }
        },
    }
}

pub fn synthesize_wav(cue: PetSoundCue, pack: SoundPack, volume: f64, pet_id: &str) -> Vec<u8> {
    let clamped_vol = if volume.is_finite() {
        volume.clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let perceptual_gain = if clamped_vol <= 0.0 {
        0.0
    } else {
        clamped_vol.powf(1.35)
    };
    let pack_trim = match pack {
        SoundPack::Healing => 0.92,
        SoundPack::Retro => 0.78,
        SoundPack::Minimal => 0.84,
    };
    let profile = voice_profile_for_pet(pet_id);
    let score = cue_score(cue);

    let tail_pad_sec = 0.025;
    let total_sec = score
        .iter()
        .map(|n| (n.start_sec + n.duration_sec) * profile.tempo_mul)
        .fold(0.08_f32, f32::max)
        + tail_pad_sec;
    let total_samples = ((total_sec * SAMPLE_RATE as f32).ceil() as usize).max(64);
    let mut mix = vec![0.0_f32; total_samples];
    let inv_sr = 1.0 / SAMPLE_RATE as f32;

    if perceptual_gain > 0.0 {
        for note in score {
            let start_sec = note.start_sec * profile.tempo_mul;
            let dur_sec = (note.duration_sec * profile.tempo_mul).max(0.025);
            let start_idx = (start_sec * SAMPLE_RATE as f32).round() as usize;
            let len_samples = (dur_sec * SAMPLE_RATE as f32).round() as usize;
            if start_idx >= total_samples || len_samples < 8 {
                continue;
            }
            let end_idx = (start_idx + len_samples).min(total_samples);
            let actual_len = end_idx - start_idx;
            let attack_sec = match pack {
                SoundPack::Retro => 0.003,
                SoundPack::Minimal => 0.0035,
                SoundPack::Healing => note.attack_sec,
            }
            .min(dur_sec * 0.35);
            let release_sec = (dur_sec * 0.26).clamp(0.008, 0.028);
            let decay_mul = match pack {
                SoundPack::Minimal => 1.38,
                SoundPack::Retro => 0.92,
                SoundPack::Healing => 1.0,
            };

            let f_start = (note.freq_start_hz * profile.pitch_mul).clamp(90.0, 6_000.0);
            let f_end = (note.freq_end_hz * profile.pitch_mul).clamp(90.0, 6_000.0);
            let mut phase = 0.0_f32;

            for i in 0..actual_len {
                let local_t = i as f32 * inv_sr;
                let progress = (local_t / dur_sec).clamp(0.0, 1.0);
                // 复古像素主题按 60Hz 寄存器步进量化音高滑音，还原掌机电子宠物颗粒感
                let pitch_t = match pack {
                    SoundPack::Retro => ((local_t * 60.0).floor() / (dur_sec * 60.0).max(1.0))
                        .clamp(0.0, 1.0)
                        .powf(note.pitch_curve),
                    SoundPack::Minimal => (progress * 0.65).powf(note.pitch_curve),
                    SoundPack::Healing => progress.powf(note.pitch_curve),
                };
                let base_freq = f_start + (f_end - f_start) * pitch_t;
                let vib = if note.vibrato_depth > 0.0 && pack != SoundPack::Minimal {
                    1.0 + note.vibrato_depth * (TAU * note.vibrato_hz * local_t).sin()
                } else {
                    1.0
                };
                let dt = (base_freq * vib * inv_sr).clamp(0.000_1, 0.45);
                let wave = sample_waveform(pack, note.role, phase, dt, local_t, profile.brightness);
                phase = (phase + dt).fract();

                let attack_env = if local_t < attack_sec && attack_sec > 0.0 {
                    0.5 * (1.0 - (std::f32::consts::PI * local_t / attack_sec).cos())
                } else {
                    1.0
                };
                let sustain_t = (local_t - attack_sec).max(0.0);
                let decay_env = (-note.decay_rate * decay_mul * sustain_t).exp();
                let remaining = dur_sec - local_t;
                let release_env = if remaining < release_sec && release_sec > 0.0 {
                    0.5 * (1.0
                        - (std::f32::consts::PI * (remaining / release_sec).clamp(0.0, 1.0)).cos())
                } else {
                    1.0
                };

                mix[start_idx + i] += wave * note.gain * attack_env * decay_env * release_env;
            }
        }
    }

    // 首尾强制平滑收零，彻底消除声卡唤醒与停止瞬间的爆音
    let fade_samples = 64.min(total_samples / 4).max(1);
    let master = perceptual_gain * pack_trim;
    let mut pcm_bytes = Vec::with_capacity(44 + total_samples * 2);
    write_wav_header(&mut pcm_bytes, total_samples);

    for (idx, &raw) in mix.iter().enumerate() {
        let edge = if idx < fade_samples {
            idx as f32 / fade_samples as f32
        } else if total_samples - 1 - idx < fade_samples {
            (total_samples - 1 - idx) as f32 / fade_samples as f32
        } else {
            1.0
        };
        let shaped = soft_limit(raw * master * edge).clamp(-1.0, 1.0);
        let sample_i16 = (shaped * f32::from(i16::MAX)).round() as i16;
        pcm_bytes.extend_from_slice(&sample_i16.to_le_bytes());
    }

    pcm_bytes
}

fn write_wav_header(out: &mut Vec<u8>, sample_count: usize) {
    let data_bytes = (sample_count * 2) as u32;
    let riff_size = 36_u32.saturating_add(data_bytes);
    let byte_rate = SAMPLE_RATE * 2;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16_u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM = 1
    out.extend_from_slice(&1_u16.to_le_bytes()); // Mono = 1 channel
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes()); // block align
    out.extend_from_slice(&16_u16.to_le_bytes()); // 16-bit
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    pack: SoundPack,
    cue: PetSoundCue,
    volume_pct: u8,
    pet_id: String,
}

#[derive(Default)]
struct AudioInner {
    last_played: HashMap<SoundCategory, Instant>,
    last_cue_at: Option<(PetSoundCue, Instant)>,
    cache: VecDeque<(CacheKey, Arc<[u8]>)>,
}

impl AudioInner {
    fn check_and_record(&mut self, cue: PetSoundCue, now: Instant) -> bool {
        let min_gap = cue.cooldown();
        let category = cue.category();
        // 高潮连招与转晕彩蛋允许抢占普通点击冷却，只防止同毫秒重复触发
        let bypass_category_gap = matches!(
            cue,
            PetSoundCue::Dizzy
                | PetSoundCue::TripleCombo
                | PetSoundCue::DoubleClick
                | PetSoundCue::Feed
                | PetSoundCue::Play
        );
        if let Some((prev_cue, prev_at)) = self.last_cue_at
            && prev_cue == cue
            && now.saturating_duration_since(prev_at) < min_gap
        {
            return false;
        }
        if !bypass_category_gap
            && let Some(&cat_at) = self.last_played.get(&category)
            && now.saturating_duration_since(cat_at) < min_gap
        {
            return false;
        }
        self.last_played.insert(category, now);
        self.last_cue_at = Some((cue, now));
        true
    }

    fn wav_for(
        &mut self,
        cue: PetSoundCue,
        pack: SoundPack,
        volume_pct: u8,
        pet_id: &str,
    ) -> Arc<[u8]> {
        if let Some(pos) = self.cache.iter().position(|(k, _)| {
            k.pack == pack && k.cue == cue && k.volume_pct == volume_pct && k.pet_id == pet_id
        }) {
            let entry = self.cache.remove(pos).unwrap();
            let wav = entry.1.clone();
            self.cache.push_back(entry);
            return wav;
        }
        let wav: Arc<[u8]> =
            synthesize_wav(cue, pack, f64::from(volume_pct) / 100.0, pet_id).into();
        if self.cache.len() >= MAX_WAV_CACHE_ENTRIES {
            self.cache.pop_front();
        }
        self.cache.push_back((
            CacheKey {
                pack,
                cue,
                volume_pct,
                pet_id: pet_id.to_owned(),
            },
            wav.clone(),
        ));
        wav
    }
}

#[derive(Default)]
pub struct AudioRuntime {
    inner: Mutex<AudioInner>,
}

impl AudioRuntime {
    pub fn play(
        &self,
        app: &tauri::AppHandle,
        cue: PetSoundCue,
        pack: SoundPack,
        volume: f64,
        pet_id: &str,
    ) -> Result<(), String> {
        self.dispatch(app, cue, pack, volume, pet_id, false)
    }

    pub fn play_preview(
        &self,
        app: &tauri::AppHandle,
        cue: PetSoundCue,
        pack: SoundPack,
        volume: f64,
        pet_id: &str,
    ) -> Result<(), String> {
        self.dispatch(app, cue, pack, volume, pet_id, true)
    }

    fn dispatch(
        &self,
        app: &tauri::AppHandle,
        cue: PetSoundCue,
        pack: SoundPack,
        volume: f64,
        pet_id: &str,
        bypass_cooldown: bool,
    ) -> Result<(), String> {
        if !volume.is_finite() || volume <= 0.005 {
            return Ok(());
        }
        let volume_pct = (volume.clamp(0.0, 1.0) * 100.0).round() as u8;
        if volume_pct == 0 {
            return Ok(());
        }
        let wav = {
            let mut guard = self.inner.lock().unwrap();
            if !bypass_cooldown && !guard.check_and_record(cue, Instant::now()) {
                return Ok(());
            }
            guard.wav_for(cue, pack, volume_pct, pet_id)
        };
        play_native_wav(app, wav)
    }
}

pub fn shutdown(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(macos_player::stop_all_on_main_thread);
    }
    #[cfg(windows)]
    {
        let _ = app;
        windows_player::stop_all();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = app;
    }
}

fn play_native_wav(app: &tauri::AppHandle, wav: Arc<[u8]>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.run_on_main_thread(move || {
            if let Err(err) = macos_player::play_wav_on_main_thread(&wav) {
                eprintln!("桌宠音效播放失败：{err}");
            }
        })
        .map_err(|e| format!("无法调度主线程音效播放：{e}"))
    }
    #[cfg(windows)]
    {
        let _ = app;
        windows_player::play_wav(wav);
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (app, wav);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos_player {
    use objc2::{
        msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyClass, AnyObject, Bool},
    };
    use std::{cell::RefCell, collections::VecDeque, ffi::c_void};

    const MAX_ACTIVE_SOUNDS: usize = 6;

    thread_local! {
        static ACTIVE_SOUNDS: RefCell<VecDeque<Retained<AnyObject>>> =
            RefCell::new(VecDeque::with_capacity(MAX_ACTIVE_SOUNDS));
    }

    pub fn play_wav_on_main_thread(wav_bytes: &[u8]) -> Result<(), String> {
        let data_cls = AnyClass::get(c"NSData").ok_or("缺少 NSData 类")?;
        let sound_cls = AnyClass::get(c"NSSound").ok_or("缺少 NSSound 类")?;
        // SAFETY: NSData 立即复制内存中的 WAV 字节；NSSound 在主线程解码合法 RIFF WAV 并异步发声，不创建窗口也不改变焦点。
        unsafe {
            let ns_data: Option<Retained<AnyObject>> = msg_send![
                data_cls,
                dataWithBytes: wav_bytes.as_ptr().cast::<c_void>(),
                length: wav_bytes.len()
            ];
            let ns_data = ns_data.ok_or("无法创建音频 NSData")?;
            let alloc: Allocated<AnyObject> = msg_send![sound_cls, alloc];
            let sound: Option<Retained<AnyObject>> = msg_send![alloc, initWithData: &*ns_data];
            let sound = sound.ok_or("无法解码 WAV 音频数据")?;

            ACTIVE_SOUNDS.with(|queue| {
                let mut queue = queue.borrow_mut();
                queue.retain(|existing| {
                    let playing: Bool = msg_send![&**existing, isPlaying];
                    playing.as_bool()
                });
                while queue.len() >= MAX_ACTIVE_SOUNDS {
                    if let Some(oldest) = queue.pop_front() {
                        let _: Bool = msg_send![&*oldest, stop];
                    }
                }
                let started: Bool = msg_send![&*sound, play];
                if started.as_bool() {
                    queue.push_back(sound);
                }
            });
        }
        Ok(())
    }

    pub fn stop_all_on_main_thread() {
        ACTIVE_SOUNDS.with(|queue| {
            let mut queue = queue.borrow_mut();
            while let Some(sound) = queue.pop_front() {
                // SAFETY: 队列仅在主线程访问，所有 NSSound 引用均有效。
                unsafe {
                    let _: Bool = msg_send![&*sound, stop];
                }
            }
        });
    }
}

#[cfg(windows)]
mod windows_player {
    use std::{
        collections::VecDeque,
        ffi::c_void,
        sync::{Arc, Mutex, OnceLock},
    };

    const SND_ASYNC: u32 = 0x0001;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_MEMORY: u32 = 0x0004;
    const MAX_RETAINED_BUFFERS: usize = 8;

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(psz_sound: *const u8, hmod: *mut c_void, fdw_sound: u32) -> i32;
    }

    static BUFFERS: OnceLock<Mutex<VecDeque<Arc<[u8]>>>> = OnceLock::new();

    pub fn play_wav(wav_bytes: Arc<[u8]>) {
        let store =
            BUFFERS.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_RETAINED_BUFFERS)));
        let ptr = wav_bytes.as_ptr();
        {
            let mut guard = store.lock().unwrap();
            if guard.len() >= MAX_RETAINED_BUFFERS {
                guard.pop_front();
            }
            guard.push_back(wav_bytes);
        }
        // SAFETY: WAV 字节由静态环形缓冲保活，SND_MEMORY | SND_ASYNC | SND_NODEFAULT 异步播放且无声卡时不触发系统错误蜂鸣。
        unsafe {
            let _ = PlaySoundW(
                ptr,
                std::ptr::null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }

    pub fn stop_all() {
        // SAFETY: 传入空指针先终止任何异步播放，再释放缓存的音频块。
        unsafe {
            let _ = PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0);
        }
        if let Some(store) = BUFFERS.get() {
            store.lock().unwrap().clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_pcm_samples(wav: &[u8]) -> Vec<i16> {
        assert!(wav.len() > 44, "WAV 数据应包含 44 字节头与非空采样");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        let riff_size = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_size + 8, wav.len());
        let sr = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        assert_eq!(sr, SAMPLE_RATE);
        let data_bytes = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(data_bytes + 44, wav.len());
        assert_eq!(data_bytes % 2, 0);
        wav[44..]
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect()
    }

    #[test]
    fn all_cues_and_sound_packs_generate_valid_anti_pop_wav_waveforms() {
        for &pack in &[SoundPack::Healing, SoundPack::Retro, SoundPack::Minimal] {
            for &cue in PetSoundCue::ALL {
                let wav = synthesize_wav(cue, pack, 0.75, "yueying-cape-dancer");
                let samples = decode_pcm_samples(&wav);
                assert!(samples.len() >= 2000, "{pack:?}/{cue:?} 采样数过短");
                let peak = samples
                    .iter()
                    .map(|&s| i32::from(s).abs())
                    .max()
                    .unwrap_or(0);
                assert!(
                    peak > 900,
                    "{pack:?}/{cue:?} 峰值过低 ({peak})，听感接近静音"
                );
                // 首尾采样必须收敛到零，防止扬声器爆音
                assert_eq!(samples[0], 0, "{pack:?}/{cue:?} 首采样未归零");
                assert_eq!(*samples.last().unwrap(), 0, "{pack:?}/{cue:?} 尾采样未归零");
            }
        }
    }

    #[test]
    fn different_sound_packs_and_pet_identities_produce_distinct_timbres() {
        let healing = synthesize_wav(
            PetSoundCue::PetHead,
            SoundPack::Healing,
            0.8,
            "yueying-cape-dancer",
        );
        let retro = synthesize_wav(
            PetSoundCue::PetHead,
            SoundPack::Retro,
            0.8,
            "yueying-cape-dancer",
        );
        let minimal = synthesize_wav(
            PetSoundCue::PetHead,
            SoundPack::Minimal,
            0.8,
            "yueying-cape-dancer",
        );
        assert_ne!(healing, retro);
        assert_ne!(healing, minimal);
        assert_ne!(retro, minimal);

        let yueying = synthesize_wav(
            PetSoundCue::Feed,
            SoundPack::Healing,
            0.8,
            "yueying-cape-dancer",
        );
        let big_fish = synthesize_wav(
            PetSoundCue::Feed,
            SoundPack::Healing,
            0.8,
            "deepseek-big-fish",
        );
        let zhizhi = synthesize_wav(
            PetSoundCue::Feed,
            SoundPack::Healing,
            0.8,
            "zhizhi-branch-guardian",
        );
        let whale_maid_profile = voice_profile_for_pet("deepseek-whale-maid");
        assert!(whale_maid_profile.pitch_mul > 1.1);
        assert!(whale_maid_profile.tempo_mul > 1.08);
        assert!(whale_maid_profile.brightness > 1.12);
        let whale_maid = synthesize_wav(
            PetSoundCue::Feed,
            SoundPack::Healing,
            0.8,
            "deepseek-whale-maid",
        );
        let whale_maid_samples = decode_pcm_samples(&whale_maid);
        assert_eq!(whale_maid_samples[0], 0);
        assert_eq!(*whale_maid_samples.last().unwrap(), 0);
        assert!(
            whale_maid_samples
                .iter()
                .any(|sample| sample.unsigned_abs() > 900)
        );
        let custom_a = synthesize_wav(PetSoundCue::Feed, SoundPack::Healing, 0.8, "custom-dragon");
        let custom_a_repeat =
            synthesize_wav(PetSoundCue::Feed, SoundPack::Healing, 0.8, "custom-dragon");
        assert_ne!(yueying, big_fish);
        assert_ne!(yueying, zhizhi);
        assert_ne!(big_fish, zhizhi);
        assert_ne!(whale_maid, yueying);
        assert_ne!(whale_maid, big_fish);
        assert_ne!(whale_maid, zhizhi);
        assert_eq!(
            custom_a, custom_a_repeat,
            "同一导入宠物的声线应保持稳定确定性"
        );
    }

    #[test]
    fn zero_volume_produces_silent_pcm_and_cooldown_guards_spam_while_allowing_combos() {
        let silent = synthesize_wav(
            PetSoundCue::TripleCombo,
            SoundPack::Healing,
            0.0,
            "yueying-cape-dancer",
        );
        let samples = decode_pcm_samples(&silent);
        assert!(samples.iter().all(|&s| s == 0));

        let mut inner = AudioInner::default();
        let t0 = Instant::now();
        assert!(inner.check_and_record(PetSoundCue::PetHead, t0));
        // 20ms 内连续同部位普通点击应被节流
        assert!(!inner.check_and_record(PetSoundCue::PokeBody, t0 + Duration::from_millis(20)));
        // 但双击、三连击、转晕彩蛋在间隔后允许突破普通点击冷却
        assert!(inner.check_and_record(PetSoundCue::DoubleClick, t0 + Duration::from_millis(40)));
        assert!(inner.check_and_record(PetSoundCue::Dizzy, t0 + Duration::from_millis(80)));

        // 自由活动轻音效具有数秒级冷却，避免频繁打扰
        assert!(inner.check_and_record(PetSoundCue::AmbientHop, t0));
        assert!(!inner.check_and_record(PetSoundCue::AmbientWave, t0 + Duration::from_millis(800)));
        assert!(
            inner.check_and_record(PetSoundCue::AmbientWave, t0 + Duration::from_millis(3_600))
        );

        // 合成缓存命中返回同一 Arc 缓冲区
        let wav1 = inner.wav_for(
            PetSoundCue::Play,
            SoundPack::Healing,
            70,
            "yueying-cape-dancer",
        );
        let wav2 = inner.wav_for(
            PetSoundCue::Play,
            SoundPack::Healing,
            70,
            "yueying-cape-dancer",
        );
        assert!(Arc::ptr_eq(&wav1, &wav2));
    }
}
