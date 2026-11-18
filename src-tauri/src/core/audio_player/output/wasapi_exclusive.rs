//! Backend WASAPI exclusive (Windows uniquement).
//!
//! Chaque piste est confiée au moteur exclusif persistant (`run_moteur_exclusif`) : le DAC
//! reste ouvert tant que le format et la sortie ne changent pas, ce qui évite 1,5 à 2,5 s
//! de réouverture à chaque morceau.

#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ringbuf::traits::Consumer;

use super::traits::{AudioOutput, AudioOutputError};
use super::types::{AudioBackend, PlaybackAtomics, SymphoniaSharedState};
use crate::core::audio_player::audio_output_wasapi::{
    default_output_device_name, run_exclusive_engine, try_negotiate_exclusive_format,
    EngineInitSignal, FluxPiste, NegotiatedFormat, WasapiSymphoniaState,
};

/// Le moteur vivant : son format, sa sortie, et la piste qu'on lui passe.
struct Moteur {
    format: NegotiatedFormat,
    sortie: Option<String>,
    prochaine: Arc<Mutex<Option<FluxPiste>>>,
    arret: Arc<AtomicBool>,
    dead: Arc<AtomicBool>,
    fil: JoinHandle<()>,
}

static MOTEUR: Mutex<Option<Moteur>> = Mutex::new(None);

fn engine_lock() -> std::sync::MutexGuard<'static, Option<Moteur>> {
    MOTEUR.lock().unwrap_or_else(|e| e.into_inner())
}

fn stop_engine(moteur: Moteur) {
    moteur.arret.store(true, Ordering::SeqCst);
    if moteur.fil.join().is_err() {
        log::warn!("WASAPI : le fil du moteur exclusif a paniqué");
    }
}

/// Rend le DAC : avant une lecture partagée, DSD ou sur une autre sortie.
pub fn close_engine() {
    let moteur = engine_lock().take();
    if let Some(m) = moteur {
        stop_engine(m);
    }
}

/// Confie une piste au moteur sous verrou unique (sans TOCTOU ni perte de `consumer` si l'init échoue).
fn attach_or_start_engine(
    format: NegotiatedFormat,
    sortie: Option<String>,
    make_flux: impl FnOnce() -> FluxPiste,
) -> Result<Arc<AtomicBool>, String> {
    let mut garde = engine_lock();
    if let Some(m) = garde.as_ref() {
        if !m.dead.load(Ordering::Acquire)
            && !m.fil.is_finished()
            && m.format == format
            && m.sortie == sortie
        {
            let dead = m.dead.clone();
            let flux = make_flux();
            *m.prochaine.lock().unwrap_or_else(|e| e.into_inner()) = Some(flux);
            log::debug!("▶️  WASAPI exclusive : moteur réutilisé ({} Hz)", format.sample_rate);
            return Ok(dead);
        }
    }
    if let Some(ancien) = garde.take() {
        stop_engine(ancien);
    }

    let prochaine = Arc::new(Mutex::new(None));
    let arret = Arc::new(AtomicBool::new(false));
    let dead = Arc::new(AtomicBool::new(false));
    let init_signal: EngineInitSignal = Arc::new((Mutex::new(None), Condvar::new()));
    let (p, a, d, sig, s) = (
        prochaine.clone(),
        arret.clone(),
        dead.clone(),
        init_signal.clone(),
        sortie.clone(),
    );
    let fil = std::thread::Builder::new()
        .name("rustmusic-wasapi-render".into())
        .spawn(move || {
            if let Err(e) = run_exclusive_engine(format, s, p, a, d, sig) {
                log::error!("❌ WASAPI exclusive : {e}");
            }
        })
        .map_err(|e| format!("spawn fil WASAPI : {e}"))?;

    // Attendre que initialize_client + start_stream réussissent AVANT de consommer `consumer`
    // (en cas d'échec, l'appelant retombe proprement sur CPAL shared).
    let (lock, cv) = &*init_signal;
    let init_res = {
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        let deadline = Instant::now() + Duration::from_secs(4);
        while g.is_none() {
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() {
                break;
            }
            let (next_g, wait_res) = cv.wait_timeout(g, rem).unwrap_or_else(|e| e.into_inner());
            g = next_g;
            if wait_res.timed_out() && g.is_none() {
                break;
            }
        }
        g.take().unwrap_or_else(|| Err("timeout ouverture stream WASAPI".into()))
    };

    if let Err(e) = init_res {
        arret.store(true, Ordering::SeqCst);
        if dead.load(Ordering::Acquire) {
            let _ = fil.join();
        }
        return Err(e);
    }

    let flux = make_flux();
    *prochaine.lock().unwrap_or_else(|e| e.into_inner()) = Some(flux);
    *garde = Some(Moteur { format, sortie, prochaine, arret, dead: dead.clone(), fil });
    Ok(dead)
}

/// Backend WASAPI exclusive pour la voie Symphonia (LiveDecode + FullBuffer).
pub struct WasapiExclusiveOutput {
    format: NegotiatedFormat,
    device_name: String,
    atomics: PlaybackAtomics,
    /// Levé par le moteur au premier tampon de cette piste.
    stream_ready: Arc<AtomicBool>,
    /// Levé si le moteur exclusif s'est terminé.
    engine_dead: Arc<AtomicBool>,
    /// Fin de cette piste pour le moteur.
    fin: Arc<AtomicBool>,
}

/// Délai maximal d'attente du premier tampon, pour ne pas figer l'interface.
const READY_TIMEOUT: Duration = Duration::from_secs(3);

/// Au-delà, on journalise le temps de réponse du DAC.
const SLOW_START: Duration = Duration::from_millis(500);

/// Rend le `Consumer` si la négociation échoue, pour un repli CPAL.
pub struct WasapiBuildError<C> {
    pub consumer: C,
    pub error: AudioOutputError,
}

impl WasapiExclusiveOutput {
    pub fn try_new<C>(
        source_sample_rate: u32,
        source_channels: u16,
        preferred_device_name: Option<String>,
        atomics: PlaybackAtomics,
        shared: SymphoniaSharedState,
        consumer: C,
    ) -> Result<Self, WasapiBuildError<C>>
    where
        C: Consumer<Item = f32> + Send + 'static,
    {
        let format = match try_negotiate_exclusive_format(source_sample_rate, source_channels, preferred_device_name.clone()) {
            Ok(f) => f,
            Err(e) => {
                return Err(WasapiBuildError { consumer, error: AudioOutputError::NegotiationFailed(e.to_string()) });
            }
        };

        let device_name = preferred_device_name
            .clone()
            .or_else(|| default_output_device_name().ok())
            .unwrap_or_else(|| "Device WASAPI inconnu".to_string());

        let stream_ready = Arc::new(AtomicBool::new(false));
        let fin = Arc::new(AtomicBool::new(false));
        let mut consumer_opt = Some(consumer);
        let engine_dead = match attach_or_start_engine(format, preferred_device_name, || FluxPiste {
            source: Box::new(consumer_opt.take().expect("consumer moved once")),
            is_paused: atomics.is_paused.clone(),
            is_stopped: atomics.is_stopped.clone(),
            fin: fin.clone(),
            volume: atomics.volume.clone(),
            current_position_frames: atomics.current_position_frames.clone(),
            seek_flush: shared.seek_flush.clone(),
            sym: WasapiSymphoniaState {
                current_source: shared.current_source.clone(),
                full_buffer_data: shared.full_buffer_data.clone(),
                full_buffer_cursor: shared.full_buffer_cursor.clone(),
                is_full_buffer_ready: shared.is_full_buffer_ready.clone(),
                pending_seek_frames: shared.pending_seek_frames.clone(),
                output_channels: shared.output_channels,
            },
            stream_ready: stream_ready.clone(),
        }) {
            Ok(d) => d,
            Err(e) => {
                let consumer = consumer_opt.take().expect("consumer unconsumed on init error");
                return Err(WasapiBuildError {
                    consumer,
                    error: AudioOutputError::StartFailed(e),
                });
            }
        };

        Ok(Self { format, device_name, atomics, stream_ready, engine_dead, fin })
    }
}

impl AudioOutput for WasapiExclusiveOutput {
    /// Attend le premier tampon réellement écrit : la lecture ne commence pas avant.
    fn start(&mut self) -> Result<(), AudioOutputError> {
        let began = Instant::now();
        while !self.stream_ready.load(Ordering::Acquire) {
            if self.atomics.is_stopped.load(Ordering::Relaxed) {
                return Ok(());
            }
            if self.engine_dead.load(Ordering::Acquire) {
                return Err(AudioOutputError::StartFailed(
                    "le moteur WASAPI exclusive s'est arrêté avant le premier tampon".into(),
                ));
            }
            if began.elapsed() >= READY_TIMEOUT {
                return Err(AudioOutputError::StartFailed(format!(
                    "WASAPI exclusive : aucun tampon écrit après {} s",
                    READY_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let waited = began.elapsed();
        if waited >= SLOW_START {
            log::warn!("WASAPI exclusive : premier tampon après {} ms", waited.as_millis());
        } else {
            log::debug!("▶️  WASAPI exclusive : son effectif après {} ms", waited.as_millis());
        }
        Ok(())
    }

    fn output_sample_rate(&self) -> u32 {
        self.format.sample_rate
    }

    fn bits_exacts(&self) -> u32 {
        u32::from(self.format.bits_per_sample).min(24)
    }

    fn output_channels(&self) -> u16 {
        self.format.channels
    }

    fn device_name(&self) -> &str {
        &self.device_name
    }

    fn backend(&self) -> AudioBackend {
        AudioBackend::WasapiExclusive
    }
}

impl Drop for WasapiExclusiveOutput {
    /// La piste s'arrête ; le moteur garde le DAC ouvert pour la suivante.
    fn drop(&mut self) {
        self.fin.store(true, Ordering::SeqCst);
        self.atomics.is_stopped.store(true, Ordering::SeqCst);
    }
}
