use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use roblox_detection::opencv::prelude::MatTraitConst;

pub(crate) struct DetectionResult {
    pub width: u32,
    pub height: u32,
    pub detection: Option<roblox_detection::Detection>,
}

#[derive(Default)]
struct InputState {
    frame: Option<roblox_runtime::graphics::CapturedFrame>,
    config: Option<roblox_detection::DetectionConfig>,
    reset: bool,
    stop: bool,
}

pub(crate) struct DetectionWorker {
    input: Arc<(Mutex<InputState>, Condvar)>,
    latest: Arc<Mutex<Option<DetectionResult>>>,
    thread: Option<JoinHandle<()>>,
    config_watcher: Option<JoinHandle<()>>,
    pending_config: Arc<Mutex<Option<roblox_detection::DetectionConfig>>>,
}

impl DetectionWorker {
    pub(crate) fn new(config: roblox_detection::DetectionConfig, config_path: PathBuf) -> Self {
        let input = Arc::new((Mutex::new(InputState::default()), Condvar::new()));
        let latest = Arc::new(Mutex::new(None));
        let pending_config = Arc::new(Mutex::new(None));
        let worker_input = Arc::clone(&input);
        let worker_latest = Arc::clone(&latest);
        let worker_config = config.clone();
        let thread = thread::Builder::new()
            .name("roblox-color-detection".into())
            .spawn(move || {
                let mut detector = roblox_detection::Detector::default();
                let mut config = worker_config;
                loop {
                    let (state_lock, wake) = &*worker_input;
                    let mut state = state_lock.lock().unwrap_or_else(|error| error.into_inner());
                    while state.frame.is_none()
                        && state.config.is_none()
                        && !state.reset
                        && !state.stop
                    {
                        state = wake.wait(state).unwrap_or_else(|error| error.into_inner());
                    }
                    if state.stop {
                        break;
                    }
                    if let Some(updated) = state.config.take() {
                        config = updated;
                        detector.reset();
                        *worker_latest
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = None;
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
        let watcher_input = Arc::clone(&input);
        let watcher_pending_config = Arc::clone(&pending_config);
        let watcher = thread::Builder::new()
            .name("roblox-detection-config".into())
            .spawn(move || {
                let mut last_bytes = std::fs::read(&config_path).ok();
                while !watcher_input
                    .0
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .stop
                {
                    thread::sleep(Duration::from_millis(500));
                    let bytes = match std::fs::read(&config_path) {
                        Ok(bytes) if Some(&bytes) != last_bytes.as_ref() => bytes,
                        Ok(_) => continue,
                        Err(_) => continue,
                    };
                    // Remember invalid content too, so a malformed edit does
                    // not emit the same parse error on every poll.
                    last_bytes = Some(bytes.clone());
                    let parsed =
                        serde_json::from_slice::<roblox_detection::DetectionConfig>(&bytes)
                            .map_err(roblox_detection::DetectionConfigError::from)
                            .and_then(|config| {
                                config.validate()?;
                                Ok(config)
                            });
                    match parsed {
                        Ok(config) => {
                            let (state_lock, wake) = &*watcher_input;
                            let mut state =
                                state_lock.lock().unwrap_or_else(|error| error.into_inner());
                            if state.stop {
                                break;
                            }
                            state.config = Some(config.clone());
                            state.reset = true;
                            *watcher_pending_config
                                .lock()
                                .unwrap_or_else(|error| error.into_inner()) = Some(config);
                            wake.notify_one();
                            eprintln!("[detection] applied updated detection.json");
                        }
                        Err(error) => {
                            eprintln!("[detection] ignoring invalid detection.json: {error}")
                        }
                    }
                }
            })
            .expect("spawn detection config watcher");
        Self {
            input,
            latest,
            thread: Some(thread),
            config_watcher: Some(watcher),
            pending_config,
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

    pub(crate) fn take_config_update(&self) -> Option<roblox_detection::DetectionConfig> {
        self.pending_config
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
        if let Some(watcher) = self.config_watcher.take() {
            let _ = watcher.join();
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
