use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use roblox_detection::opencv::prelude::MatTraitConst;

pub(crate) struct DetectionResult {
    pub width: u32,
    pub height: u32,
    pub detection: Option<roblox_detection::Detection>,
}

#[derive(Default)]
struct InputState {
    frame: Option<roblox_runtime::graphics::CapturedFrame>,
    reset: bool,
    stop: bool,
}

pub(crate) struct DetectionWorker {
    input: Arc<(Mutex<InputState>, Condvar)>,
    latest: Arc<Mutex<Option<DetectionResult>>>,
    thread: Option<JoinHandle<()>>,
}

impl DetectionWorker {
    pub(crate) fn new(config: roblox_detection::DetectionConfig) -> Self {
        let input = Arc::new((Mutex::new(InputState::default()), Condvar::new()));
        let latest = Arc::new(Mutex::new(None));
        let worker_input = Arc::clone(&input);
        let worker_latest = Arc::clone(&latest);
        let thread = thread::Builder::new()
            .name("roblox-color-detection".into())
            .spawn(move || {
                let mut detector = roblox_detection::Detector::default();
                loop {
                    let (state_lock, wake) = &*worker_input;
                    let mut state = state_lock.lock().unwrap_or_else(|error| error.into_inner());
                    while state.frame.is_none() && !state.reset && !state.stop {
                        state = wake.wait(state).unwrap_or_else(|error| error.into_inner());
                    }
                    if state.stop {
                        break;
                    }
                    if state.reset {
                        detector.reset();
                        state.reset = false;
                        *worker_latest
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = None;
                    }
                    let frame = state.frame.take();
                    drop(state);

                    let Some(frame) = frame else { continue };
                    let detection = detect_frame(&mut detector, &config, &frame);
                    *worker_latest
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(DetectionResult {
                        width: frame.width,
                        height: frame.height,
                        detection,
                    });
                }
            })
            .expect("spawn color detection worker");
        Self {
            input,
            latest,
            thread: Some(thread),
        }
    }

    /// Keep only the newest frame when detection falls behind presentation.
    pub(crate) fn submit(&self, frame: roblox_runtime::graphics::CapturedFrame) {
        let (state_lock, wake) = &*self.input;
        let mut state = state_lock.lock().unwrap_or_else(|error| error.into_inner());
        state.frame = Some(frame);
        wake.notify_one();
    }

    pub(crate) fn reset(&self) {
        let (state_lock, wake) = &*self.input;
        let mut state = state_lock.lock().unwrap_or_else(|error| error.into_inner());
        state.frame = None;
        state.reset = true;
        wake.notify_one();
    }

    pub(crate) fn take_latest(&self) -> Option<DetectionResult> {
        self.latest
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

impl Drop for DetectionWorker {
    fn drop(&mut self) {
        let (state_lock, wake) = &*self.input;
        let mut state = state_lock.lock().unwrap_or_else(|error| error.into_inner());
        state.stop = true;
        wake.notify_one();
        drop(state);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn detect_frame(
    detector: &mut roblox_detection::Detector,
    config: &roblox_detection::DetectionConfig,
    frame: &roblox_runtime::graphics::CapturedFrame,
) -> Option<roblox_detection::Detection> {
    let row_bytes = (frame.width as usize).checked_mul(3)?;
    let expected_bytes = row_bytes.checked_mul(frame.height as usize)?;
    if frame.width == 0 || frame.height == 0 || frame.bgr.len() != expected_bytes {
        return None;
    }
    let flat = roblox_detection::opencv::core::Mat::from_slice(&frame.bgr).ok()?;
    let bgr_view = flat.reshape(3, frame.height as i32).ok()?;
    let mut bgr = roblox_detection::opencv::core::Mat::default();
    bgr_view.copy_to(&mut bgr).ok()?;
    match detector.detect(&bgr, config) {
        Ok(detection) => detection,
        Err(error) => {
            eprintln!("[detection] OpenCV failed to process frame: {error}");
            None
        }
    }
}
