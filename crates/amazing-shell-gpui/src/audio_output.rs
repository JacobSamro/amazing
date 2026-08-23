#![allow(unsafe_code)]

use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Media::Audio::{
    CALLBACK_NULL, HWAVEOUT, WAVE_FORMAT_PCM, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR, WHDR_DONE,
    waveOutClose, waveOutGetPosition, waveOutOpen, waveOutPrepareHeader, waveOutUnprepareHeader,
    waveOutWrite,
};
use windows_sys::Win32::Media::{MMSYSERR_NOERROR, MMTIME, TIME_BYTES, TIME_SAMPLES};

#[derive(Clone)]
pub struct AudioDeviceClock {
    cursor_frames: Arc<AtomicU64>,
    active: Arc<AtomicBool>,
    sample_rate: u32,
    total_frames: u64,
}

impl AudioDeviceClock {
    pub fn start(sample_rate: u32, channels: u16, samples: Vec<i16>) -> Result<Self, String> {
        validate_track(sample_rate, channels, &samples)?;
        let total_frames = samples.len() as u64 / channels as u64;
        let cursor_frames = Arc::new(AtomicU64::new(0));
        let active = Arc::new(AtomicBool::new(false));
        let thread_cursor = Arc::clone(&cursor_frames);
        let thread_active = Arc::clone(&active);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("amazing-waveout".into())
            .spawn(move || {
                run_wave_output(
                    sample_rate,
                    channels,
                    samples,
                    thread_cursor,
                    thread_active,
                    ready_sender,
                );
            })
            .map_err(|error| format!("spawn Windows audio output: {error}"))?;
        ready_receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows audio output startup timed out".to_string())??;
        Ok(Self {
            cursor_frames,
            active,
            sample_rate,
            total_frames,
        })
    }

    pub fn position_ms(&self) -> u64 {
        samples_to_ms(self.cursor_frames.load(Ordering::Acquire), self.sample_rate)
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn total_ms(&self) -> u64 {
        samples_to_ms(self.total_frames, self.sample_rate)
    }
}

fn validate_track(sample_rate: u32, channels: u16, samples: &[i16]) -> Result<(), String> {
    if !(8_000..=192_000).contains(&sample_rate)
        || !matches!(channels, 1 | 2)
        || samples.is_empty()
        || !samples.len().is_multiple_of(channels as usize)
        || samples.len().saturating_mul(std::mem::size_of::<i16>()) > u32::MAX as usize
    {
        return Err("Windows PCM output track is invalid".into());
    }
    Ok(())
}

fn samples_to_ms(sample_frames: u64, sample_rate: u32) -> u64 {
    sample_frames.saturating_mul(1_000) / sample_rate.max(1) as u64
}

fn run_wave_output(
    sample_rate: u32,
    channels: u16,
    samples: Vec<i16>,
    cursor_frames: Arc<AtomicU64>,
    active: Arc<AtomicBool>,
    ready_sender: mpsc::SyncSender<Result<(), String>>,
) {
    let block_align = channels.saturating_mul(std::mem::size_of::<i16>() as u16);
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: channels,
        nSamplesPerSec: sample_rate,
        nAvgBytesPerSec: sample_rate.saturating_mul(block_align as u32),
        nBlockAlign: block_align,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut handle: HWAVEOUT = null_mut();
    // SAFETY: `handle` and `format` are valid for the duration of the call;
    // CALLBACK_NULL means WinMM stores no callback pointer or caller instance.
    let open_result =
        unsafe { waveOutOpen(&mut handle, WAVE_MAPPER, &format, 0, 0, CALLBACK_NULL) };
    if open_result != MMSYSERR_NOERROR {
        let _ = ready_sender.send(Err(format!("waveOutOpen failed: {open_result}")));
        return;
    }

    let mut samples = samples.into_boxed_slice();
    let mut header = Box::new(WAVEHDR {
        lpData: samples.as_mut_ptr().cast(),
        dwBufferLength: (samples.len() * std::mem::size_of::<i16>()) as u32,
        ..Default::default()
    });
    let header_size = std::mem::size_of::<WAVEHDR>() as u32;
    // SAFETY: the device handle is open, and the boxed header plus boxed sample
    // buffer remain at stable addresses until unprepare and close complete.
    let prepare_result = unsafe { waveOutPrepareHeader(handle, &mut *header, header_size) };
    if prepare_result != MMSYSERR_NOERROR {
        // SAFETY: `handle` was returned by waveOutOpen and no buffer was queued.
        let _ = unsafe { waveOutClose(handle) };
        let _ = ready_sender.send(Err(format!(
            "waveOutPrepareHeader failed: {prepare_result}"
        )));
        return;
    }
    // SAFETY: the header was prepared for this handle and its PCM buffer stays
    // alive and immovable until WinMM marks the header done.
    let write_result = unsafe { waveOutWrite(handle, &mut *header, header_size) };
    if write_result != MMSYSERR_NOERROR {
        // SAFETY: the prepared header was not successfully queued; it is valid
        // to unprepare it before closing the associated handle.
        let _ = unsafe { waveOutUnprepareHeader(handle, &mut *header, header_size) };
        // SAFETY: `handle` is still the open handle returned by waveOutOpen.
        let _ = unsafe { waveOutClose(handle) };
        let _ = ready_sender.send(Err(format!("waveOutWrite failed: {write_result}")));
        return;
    }
    active.store(true, Ordering::Release);
    let _ = ready_sender.send(Ok(()));

    loop {
        let mut position = MMTIME {
            wType: TIME_SAMPLES,
            ..Default::default()
        };
        // SAFETY: `handle` remains open and `position` points to writable MMTIME
        // storage of the exact size supplied to WinMM.
        if unsafe {
            waveOutGetPosition(handle, &mut position, std::mem::size_of::<MMTIME>() as u32)
        } == MMSYSERR_NOERROR
        {
            // SAFETY: WinMM selects the active union field via `wType`; both
            // supported branches below read only their corresponding field.
            let frames = unsafe {
                match position.wType {
                    TIME_SAMPLES => position.u.sample as u64,
                    TIME_BYTES => position.u.cb as u64 / block_align.max(1) as u64,
                    _ => cursor_frames.load(Ordering::Relaxed),
                }
            };
            cursor_frames.store(frames, Ordering::Release);
        }
        if header.dwFlags & WHDR_DONE != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cursor_frames.store(samples.len() as u64 / channels as u64, Ordering::Release);
    active.store(false, Ordering::Release);
    // SAFETY: WHDR_DONE proves WinMM has returned the queued stable buffer; the
    // header is unprepared before either allocation can be freed.
    let _ = unsafe { waveOutUnprepareHeader(handle, &mut *header, header_size) };
    // SAFETY: all prepared buffers have been returned and unprepared.
    let _ = unsafe { waveOutClose(handle) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_cursor_conversion_is_exact_and_bounded() {
        assert_eq!(samples_to_ms(0, 48_000), 0);
        assert_eq!(samples_to_ms(24_000, 48_000), 500);
        assert_eq!(samples_to_ms(48_000 * 30, 48_000), 30_000);
        assert_eq!(samples_to_ms(u64::MAX, 48_000), u64::MAX / 48_000);
    }

    #[test]
    fn audio_track_validation_rejects_partial_or_unsupported_pcm() {
        assert!(validate_track(48_000, 1, &[1, 2]).is_ok());
        assert!(validate_track(7_999, 1, &[1]).is_err());
        assert!(validate_track(48_000, 3, &[1, 2, 3]).is_err());
        assert!(validate_track(48_000, 2, &[1]).is_err());
    }
}
