//! Orchestrates DSD playback. Reads DSF blocks → converts to PCM →
//! optionally resamples to the device output rate → pushes interleaved
//! PCM frames into the player's RingBuffer producer.
//!
//! Mirrors the contract of `audio_player::decode_thread` (the Symphonia
//! path): it reacts to the same shared atomics for stop, seek, position,
//! and total duration. The player just spawns a thread that calls this
//! function — no DSD-specific knowledge leaks back into `audio_player`.
//!
//! DSD playback always runs in LiveDecode mode (CPAL reads from the ring
//! buffer). The FullBuffer optimization used by Symphonia for instant seek
//! is not applied here (a 4-min stereo DSD64 in RAM is ~80 MB; not worth it
//! for the seek-instantaneity gain).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use ringbuf::traits::Producer;

use crate::core::audio_decoder::dsd::dsd_container::DsdContainerReader;
use crate::core::audio_decoder::dsd::dsd_converter::DsdToPcmConverter;
use crate::core::audio_decoder::error::DecodeError;
use crate::core::audio_player::audio_utils::adapt_channels;
use crate::core::audio_quality;
use crate::core::audio_resampler::resampler::Resampler;

/// Run the full DSD playback loop on the calling thread.
/// Spawn this from the player via `std::thread::spawn`.
///
/// Takes a `Box<dyn DsdContainerReader + Send>` so the same loop drives
/// both `DsfDecoder` (Sony DSF) and `DffDecoder` (Philips DSDIFF) — the
/// dispatch on extension happens at the call site.
pub fn run_dsd_playback<P>(
    mut decoder: Box<dyn DsdContainerReader + Send>,
    output_sample_rate: u32,
    output_channels: u16,
    mut producer: P,
    is_stopped: Arc<AtomicBool>,
    seek_position: Arc<AtomicU64>,
    current_position_frames: Arc<AtomicUsize>,
    total_duration: Arc<AtomicU64>,
    seek_flush: Arc<AtomicBool>,
    cue_start_secs: Arc<AtomicU64>,
    cue_end_secs: Arc<AtomicU64>,
) -> Result<(), DecodeError>
where
    P: Producer<Item = f32>,
{
    let dsd_rate = decoder.sample_rate();
    let channels = decoder.channel_count() as usize;

    // Pick filter window + target PCM rate from the current quality profile.
    // High/Medium use the audiophile path (88.2 kHz, 2048/1024 taps); Low
    // drops to 44.1 kHz with 512 taps — about 4× cheaper CPU-wise, audible
    // only to expert ears on hi-fi gear.
    let profile = audio_quality::current_profile();
    let intermediate_pcm_rate = profile.dsd_target_rate(dsd_rate);
    let window_bytes = profile.dsd_filter_window_bytes();

    log::debug!(
        "🧵 DSD playback démarré ({} ch, DSD {} Hz → PCM {} Hz → device {} Hz × {} ch, profile {:?})",
        channels,
        dsd_rate,
        intermediate_pcm_rate,
        output_sample_rate,
        output_channels,
        profile,
    );

    // Build DSD → PCM converter (LUT + filter, sized per profile)
    let mut converter = DsdToPcmConverter::new_with_window(
        dsd_rate,
        intermediate_pcm_rate,
        channels as u8,
        window_bytes,
    )?;

    // Build PCM → device-rate resampler (None if intermediate already matches device)
    let mut resampler = Resampler::maybe_new_with_profile(
        intermediate_pcm_rate,
        output_sample_rate,
        channels,
        profile,
    )
    .map_err(DecodeError::InvalidConfig)?;

    let need_channel_adapt = channels as u16 != output_channels;
    let out_ch = output_channels.max(1) as usize;

    let cue_start_init = f64::from_bits(cue_start_secs.load(Ordering::Relaxed)).max(0.0);
    let mut cur_output_frames: u64 = if cue_start_init > 0.0 {
        let actual_secs = decoder.seek_to_seconds(cue_start_init)?;
        let f = (actual_secs * output_sample_rate as f64).round() as u64;
        current_position_frames.store(f as usize, Ordering::Relaxed);
        f
    } else {
        0
    };

    loop {
        // ─── Stop ───
        if is_stopped.load(Ordering::Relaxed) {
            log::debug!("DSD: stop détecté");
            break;
        }

        // ─── Seek ───
        let seek_bits = seek_position.load(Ordering::Relaxed);
        if seek_bits != u64::MAX {
            seek_position.store(u64::MAX, Ordering::Relaxed);

            let total_dur = f64::from_bits(total_duration.load(Ordering::Relaxed));
            let cue_start = f64::from_bits(cue_start_secs.load(Ordering::Relaxed)).max(0.0);
            let cue_end = f64::from_bits(cue_end_secs.load(Ordering::Relaxed)).max(0.0);
            let max_seek = if cue_end > cue_start {
                cue_end
            } else {
                cue_start + total_dur.max(0.0)
            };
            let min_seek = if cue_end > cue_start { cue_start } else { 0.0 };
            let seek_seconds = f64::from_bits(seek_bits).clamp(min_seek, max_seek);

            match decoder.seek_to_seconds(seek_seconds) {
                Ok(actual_seconds) => {
                    converter.reset();
                    if let Some(ref mut rs) = resampler {
                        rs.reset();
                    }

                    let new_frames =
                        (actual_seconds * output_sample_rate as f64) as usize;
                    cur_output_frames = new_frames as u64;
                    current_position_frames.store(new_frames, Ordering::Relaxed);

                    // Flush ring buffer — wait que CPAL ACK le flush (5-20 ms
                    // en conditions normales). Poll 1 ms, timeout 80 ms.
                    seek_flush.store(true, Ordering::Relaxed);
                    let flush_start = std::time::Instant::now();
                    while seek_flush.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        if flush_start.elapsed().as_millis() > 80 {
                            seek_flush.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                    log::debug!("DSD seek à {:.1}s", actual_seconds);
                }
                Err(e) => {
                    log::error!("DSD seek failed: {:?}", e);
                }
            }
            continue;
        }

        // ─── Read next DSD super-block ───
        let blocks = match decoder.read_next_blocks()? {
            Some(b) => b,
            None => {
                log::debug!("DSD: fin du fichier");
                break;
            }
        };

        // ─── Convert each channel to PCM (parallèle si 3+ canaux) ───
        // En stéréo : séquentiel (overhead rayon pas justifié).
        // En multicanal (SACD 5.0/5.1) : 1 thread par canal → speedup quasi-linéaire,
        // c'est ce qui rend le DSD64 5-canaux jouable en profil High sur CPU normal.
        let per_channel_pcm: Vec<Vec<f32>> = converter.process_blocks(&blocks);

        let n_samples = per_channel_pcm[0].len();
        if n_samples == 0 {
            continue;
        }

        // ─── Interleave per-channel into [s0_ch0, s0_ch1, s1_ch0, s1_ch1, ...] ───
        let mut interleaved: Vec<f32> = Vec::with_capacity(n_samples * channels);
        for i in 0..n_samples {
            for ch in 0..channels {
                interleaved.push(per_channel_pcm[ch][i]);
            }
        }

        // ─── Resample to device rate (no-op if rates match) ───
        let resampled: Vec<f32> = match resampler.as_mut() {
            Some(rs) => rs.process_interleaved(&interleaved),
            None => interleaved,
        };

        if resampled.is_empty() {
            continue;
        }

        // ─── Channel adaptation (e.g. stereo → 5.1) ───
        let final_samples: Vec<f32> = if need_channel_adapt {
            let frames = resampled.len() / channels;
            let mut out = vec![0.0f32; frames * output_channels as usize];
            adapt_channels(&resampled, channels, &mut out, output_channels.into());
            out
        } else {
            resampled
        };

        let cue_start = f64::from_bits(cue_start_secs.load(Ordering::Relaxed)).max(0.0);
        let cue_end = f64::from_bits(cue_end_secs.load(Ordering::Relaxed)).max(0.0);
        let frames_here = final_samples.len() / out_ch;
        let mut push_slice = final_samples.as_slice();
        let mut reached_cue_end = false;
        if cue_end > cue_start {
            let end_frame_target = (cue_end * output_sample_rate as f64).round() as u64;
            if cur_output_frames >= end_frame_target {
                break;
            }
            if cur_output_frames.saturating_add(frames_here as u64) >= end_frame_target {
                let keep_frames = end_frame_target.saturating_sub(cur_output_frames) as usize;
                push_slice = &final_samples[..keep_frames * out_ch];
                reached_cue_end = true;
            }
        }
        cur_output_frames = cur_output_frames.saturating_add((push_slice.len() / out_ch) as u64);

        // ─── Push into ring buffer (block when full) ───
        // ⚡ Réactivité seek : on abandonne le push en cours si un seek est
        // demandé pendant qu'on est bloqué sur un ring buffer plein (sinon
        // latence ressentie à chaque seek de 20-100 ms).
        let mut offset = 0usize;
        while offset < push_slice.len() {
            if is_stopped.load(Ordering::Relaxed) {
                break;
            }
            if seek_position.load(Ordering::Relaxed) != u64::MAX {
                break;
            }
            let written = producer.push_slice(&push_slice[offset..]);
            offset += written;
            if written == 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        if reached_cue_end {
            break;
        }
    }

    log::debug!("🧵 DSD playback thread terminé");
    Ok(())
}
