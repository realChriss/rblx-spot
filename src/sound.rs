use std::f32::consts::TAU;
use std::ptr::null_mut;

use windows_sys::Win32::Media::Audio::*;

pub fn play_startup() {
    std::thread::spawn(|| {
        let sound = startup_sound(); // SND_SYNC keeps the buffer alive; ASYNC would read freed memory
        unsafe { PlaySoundW(sound.as_ptr().cast(), null_mut(), SND_MEMORY | SND_SYNC | SND_NODEFAULT) };
    });
}

fn sine(f: f32, t: f32) -> f32 {
    (TAU * f * t).sin()
}

fn env(t: f32, at: f32, attack: f32, decay: f32) -> f32 {
    let t = t - at;
    if t < 0.0 { 0.0 } else { (t / attack).min(1.0) * (-t * decay).exp() }
}

fn glide(t: f32, f0: f32, f1: f32, k: f32) -> f32 {
    (TAU * (f1 * t + (f0 - f1) * (1.0 - (-k * t).exp()) / k)).sin()
}

fn reverb(dry: &[f32], spread: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; dry.len()];
    for d in [1116, 1188, 1277, 1356] {
        let d = d + spread;
        let (mut buf, mut lp) = (vec![0.0f32; d], 0.0f32);
        for (i, o) in out.iter_mut().enumerate() {
            let y = buf[i % d];
            lp = y * 0.7 + lp * 0.3;
            buf[i % d] = dry[i] + lp * 0.82;
            *o += y;
        }
    }
    for d in [556, 441] {
        let d = d + spread;
        let mut buf = vec![0.0f32; d];
        for (i, o) in out.iter_mut().enumerate() {
            let b = buf[i % d];
            buf[i % d] = *o + b * 0.5;
            *o = b - *o;
        }
    }
    out
}

fn wav(len: f32, wet: f32, f: impl Fn(f32) -> f32) -> Vec<u8> {
    const RATE: u32 = 44100;
    const TAIL: f32 = 1.0;
    let n = ((len + TAIL) * RATE as f32) as usize;
    let dry: Vec<f32> = (0..n)
        .map(|i| i as f32 / RATE as f32)
        .map(|t| if t < len { f(t) * ((len - t) / 0.02).min(1.0) } else { 0.0 })
        .collect();
    let peak = |x: &[f32]| x.iter().fold(1e-9f32, |m, v| m.max(v.abs()));
    let (l, r) = (reverb(&dry, 0), reverb(&dry, 23));
    let wet = wet * peak(&dry) / peak(&l).max(peak(&r));
    let fade = |i: usize| ((n - i) as f32 / (0.15 * RATE as f32)).min(1.0);
    let frames: Vec<f32> = (0..n).flat_map(|i| [dry[i] + wet * l[i], dry[i] + wet * r[i]].map(|x| x * fade(i))).collect();
    let gain = 0.5 / peak(&frames);
    let data = 2 * frames.len() as u32;
    let header: [&[u8]; 12] = [
        b"RIFF", &(36 + data).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(),
        &2u16.to_le_bytes(), &RATE.to_le_bytes(), &(RATE * 4).to_le_bytes(), &4u16.to_le_bytes(),
        &16u16.to_le_bytes(), b"data", &data.to_le_bytes(),
    ];
    let mut out = header.concat();
    for x in frames {
        out.extend_from_slice(&((x * gain * 32767.0) as i16).to_le_bytes());
    }
    out
}

fn startup_sound() -> Vec<u8> {
    wav(0.5, 0.3, |t| {
        0.8 * env(t, 0.0, 0.002, 25.0) * glide(t, 160.0, 60.0, 30.0)
            + [659.3, 987.8].iter().map(|&f| env(t, 0.0, 0.003, 10.0) * sine(f, t)).sum::<f32>()
    })
}
