//! Linux joydev controller input forwarded through Roblox's GameActivity JNI
//! gamepad natives. Xbox and PlayStation pads use the same Linux button and
//! axis interface; Roblox's glyph type remains configurable because its
//! integer values are not published by the runtime.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

const EVENT_BUTTON: u8 = 1;
const EVENT_AXIS: u8 = 2;
const EVENT_INIT: u8 = 0x80;
const EVENT_SIZE: usize = 8;
const NONBLOCK: i32 = 0o4000;
const MAX_PADS: usize = 4;
const RESCAN_AFTER: Duration = Duration::from_secs(2);
const ACTIVE_PAD_POLL_MS: u64 = 8;
const SOURCE_GAMEPAD_JOYSTICK: i32 = 0x0000_0401 | 0x0100_0010;
const ACTION_DOWN: i32 = 0;
const ACTION_UP: i32 = 1;

const ABS_X: usize = 0x00;
const ABS_Y: usize = 0x01;
const ABS_Z: usize = 0x02;
const ABS_RX: usize = 0x03;
const ABS_RY: usize = 0x04;
const ABS_RZ: usize = 0x05;
const ABS_HAT0X: usize = 0x10;
const ABS_HAT0Y: usize = 0x11;
const BTN_MISC: usize = 0x100;
const BTN_JOYSTICK: usize = 0x120;
const BTN_THUMBR: usize = 0x13f;

const AXIS_X: i32 = 0;
const AXIS_Y: i32 = 1;
const AXIS_Z: i32 = 11;
const AXIS_RZ: i32 = 14;
const AXIS_HAT_X: i32 = 15;
const AXIS_HAT_Y: i32 = 16;
const AXIS_LTRIGGER: i32 = 17;
const AXIS_RTRIGGER: i32 = 18;

#[derive(Default)]
struct Natives {
    connect: Option<*mut std::ffi::c_void>,
    disconnect: Option<*mut std::ffi::c_void>,
    button: Option<*mut std::ffi::c_void>,
    axis: Option<*mut std::ffi::c_void>,
    supported_key: Option<*mut std::ffi::c_void>,
    supported_motion: Option<*mut std::ffi::c_void>,
}

impl Natives {
    fn from_engine(engine: &roblox_runtime::LoadedEngine) -> Self {
        let get = |name| engine.symbol(name);
        Self {
            connect: get(
                "Java_com_roblox_engine_jni_NativeInputInterface_nativeGamepadConnectEventWithGamepadType",
            ),
            disconnect: get(
                "Java_com_roblox_engine_jni_NativeInputInterface_nativeGamepadDisconnectEvent",
            ),
            button: get("Java_com_roblox_engine_jni_NativeInputInterface_nativeGamepadButtonEvent"),
            axis: get("Java_com_roblox_engine_jni_NativeInputInterface_nativeGamepadAxisEvent"),
            supported_key: get(
                "Java_com_roblox_engine_jni_NativeInputInterface_nativeSetGamepadSupportedKeyWithGamepadType",
            ),
            supported_motion: get(
                "Java_com_roblox_engine_jni_NativeInputInterface_nativeSetGamepadSupportedMotionWithGamepadType",
            ),
        }
    }

    fn ready(&self) -> bool {
        self.connect.is_some()
            && self.disconnect.is_some()
            && self.button.is_some()
            && self.axis.is_some()
            && self.supported_key.is_some()
            && self.supported_motion.is_some()
    }

    fn connect(&self, id: i32, kind: i32) -> Result<(), String> {
        let native = self.connect.ok_or("gamepad connect native is missing")?;
        // SAFETY: symbols are resolved from the live libroblox.so and the
        // runtime's JNI trampoline validates the active process VM.
        unsafe { roblox_runtime::jni::game_activity::gamepad_connect(native, id, kind) }
    }

    fn disconnect(&self, id: i32) -> Result<(), String> {
        let native = self
            .disconnect
            .ok_or("gamepad disconnect native is missing")?;
        // SAFETY: same live-library and process-VM invariant as `connect`.
        unsafe { roblox_runtime::jni::game_activity::gamepad_disconnect(native, id) }
    }

    fn button(&self, id: i32, code: i32, down: bool) -> Result<(), String> {
        let native = self.button.ok_or("gamepad button native is missing")?;
        // SAFETY: same live-library and process-VM invariant as `connect`.
        unsafe {
            roblox_runtime::jni::game_activity::gamepad_button(
                native,
                id,
                code,
                if down { ACTION_DOWN } else { ACTION_UP },
            )
        }
    }

    fn axis(&self, id: i32, axis: i32, x: f32, y: f32, z: f32) -> Result<(), String> {
        let native = self.axis.ok_or("gamepad axis native is missing")?;
        // SAFETY: same live-library and process-VM invariant as `connect`.
        unsafe { roblox_runtime::jni::game_activity::gamepad_axis(native, id, axis, x, y, z) }
    }

    fn supported_key(&self, id: i32, key: i32, kind: i32) -> Result<(), String> {
        let native = self
            .supported_key
            .ok_or("gamepad supported-key native is missing")?;
        // SAFETY: same live-library and process-VM invariant as `connect`.
        unsafe {
            roblox_runtime::jni::game_activity::gamepad_supported_key(native, id, key, true, kind)
        }
    }

    fn supported_motion(&self, id: i32, axis: i32, kind: i32) -> Result<(), String> {
        let native = self
            .supported_motion
            .ok_or("gamepad supported-motion native is missing")?;
        // SAFETY: same live-library and process-VM invariant as `connect`.
        unsafe {
            roblox_runtime::jni::game_activity::gamepad_supported_motion(
                native,
                id,
                axis,
                SOURCE_GAMEPAD_JOYSTICK,
                true,
                kind,
            )
        }
    }
}

pub(crate) struct GamepadHost {
    natives: Natives,
    pads: Vec<Pad>,
    last_scan: Instant,
    enabled: bool,
    gamepad_type: i32,
}

impl GamepadHost {
    pub(crate) fn new(engine: &roblox_runtime::LoadedEngine) -> Self {
        let natives = Natives::from_engine(engine);
        let enabled = !matches!(std::env::var("RUSTY_BLOX_GAMEPAD").as_deref(), Ok("0"));
        let gamepad_type = std::env::var("RUSTY_BLOX_GAMEPAD_TYPE")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0);
        if !enabled {
            eprintln!("rusty-blox: controller support disabled by RUSTY_BLOX_GAMEPAD=0");
        } else if !natives.ready() {
            eprintln!(
                "rusty-blox: controller support unavailable; Roblox did not export all six gamepad natives"
            );
        } else {
            eprintln!(
                "rusty-blox: Linux controller input enabled (gamepad type {gamepad_type}; glyph mapping unverified)"
            );
        }
        Self {
            natives,
            pads: Vec::new(),
            last_scan: Instant::now() - RESCAN_AFTER,
            enabled,
            gamepad_type,
        }
    }

    pub(crate) fn poll(&mut self) {
        if !self.enabled || !self.natives.ready() {
            return;
        }
        if self.last_scan.elapsed() >= RESCAN_AFTER {
            self.last_scan = Instant::now();
            self.scan();
        }

        let mut gone = Vec::new();
        for pad in &mut self.pads {
            if !pad.drain(&self.natives, self.gamepad_type) {
                gone.push(pad.id);
            }
        }
        for id in gone {
            if let Some(pad) = self.pads.iter().find(|pad| pad.id == id) {
                if pad.announced {
                    if let Err(error) = self.natives.disconnect(id) {
                        eprintln!("rusty-blox: disconnect controller {id}: {error}");
                    }
                }
            }
            self.pads.retain(|pad| pad.id != id);
        }
    }

    pub(crate) fn disconnect_all(&mut self) {
        for pad in self.pads.drain(..).filter(|pad| pad.announced) {
            if let Err(error) = self.natives.disconnect(pad.id) {
                eprintln!("rusty-blox: disconnect controller {}: {error}", pad.id);
            }
        }
    }

    pub(crate) fn poll_interval_ms(&self) -> u64 {
        if self.enabled && !self.pads.is_empty() {
            ACTIVE_PAD_POLL_MS
        } else {
            16
        }
    }

    fn scan(&mut self) {
        let Ok(entries) = fs::read_dir("/dev/input") else {
            return;
        };
        let mut nodes: Vec<i32> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().and_then(js_node))
            .collect();
        nodes.sort_unstable();
        for node in nodes {
            if self.pads.len() >= MAX_PADS {
                break;
            }
            if self.pads.iter().any(|pad| pad.node == node) || !is_gamepad(node) {
                continue;
            }
            let Some(file) = open_pad(node) else { continue };
            let id = (0..MAX_PADS as i32)
                .find(|id| !self.pads.iter().any(|pad| pad.id == *id))
                .unwrap_or(0);
            let name = device_name(node).unwrap_or_else(|| "unknown controller".into());
            eprintln!(
                "rusty-blox: controller {id} connected: {name}; gamepad type {} (unverified)",
                self.gamepad_type
            );
            self.pads.push(Pad {
                id,
                node,
                file,
                layout: read_layout(node),
                state: AxisState::default(),
                announced: false,
            });
        }
    }
}

struct Pad {
    id: i32,
    node: i32,
    file: File,
    layout: Layout,
    state: AxisState,
    announced: bool,
}

impl Pad {
    fn drain(&mut self, natives: &Natives, kind: i32) -> bool {
        let mut packet = [0_u8; EVENT_SIZE];
        let mut button_count = 0_u8;
        let mut axis_count = 0_u8;
        let mut pending = Vec::new();
        loop {
            match self.file.read(&mut packet) {
                Ok(EVENT_SIZE) => {
                    let (event_kind, index, value) = decode(&packet);
                    if event_kind & EVENT_INIT != 0 {
                        match event_kind & !EVENT_INIT {
                            EVENT_BUTTON => {
                                button_count = button_count.max(index.saturating_add(1))
                            }
                            EVENT_AXIS => axis_count = axis_count.max(index.saturating_add(1)),
                            _ => {}
                        }
                    } else if self.announced {
                        self.dispatch(natives, event_kind, index, value);
                    } else {
                        pending.push((event_kind, index, value));
                    }
                }
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        if !self.announced && (button_count > 0 || axis_count > 0) {
            if let Err(error) = natives.connect(self.id, kind) {
                eprintln!("rusty-blox: announce controller {}: {error}", self.id);
                return false;
            }
            for index in 0..button_count {
                if let Some(code) = self.layout.buttons.get(index as usize).copied().flatten() {
                    if let Err(error) = natives.supported_key(self.id, code, kind) {
                        eprintln!("rusty-blox: declare controller button {code}: {error}");
                        report(natives.disconnect(self.id));
                        return false;
                    }
                }
            }
            for index in 0..axis_count {
                if let Some(axis) = self.layout.axes.get(index as usize).copied().flatten() {
                    if let Err(error) = natives.supported_motion(self.id, axis, kind) {
                        eprintln!("rusty-blox: declare controller axis {axis}: {error}");
                        report(natives.disconnect(self.id));
                        return false;
                    }
                }
            }
            self.announced = true;
            for (event_kind, index, value) in pending {
                self.dispatch(natives, event_kind, index, value);
            }
        }
        true
    }

    fn dispatch(&mut self, natives: &Natives, event_kind: u8, index: u8, value: i16) {
        match event_kind & !EVENT_INIT {
            EVENT_BUTTON => {
                if let Some(code) = self.layout.buttons.get(index as usize).copied().flatten() {
                    report(natives.button(self.id, code, value != 0));
                }
            }
            EVENT_AXIS => {
                if let Some(axis) = self.layout.axes.get(index as usize).copied().flatten() {
                    if let Some((code, x, y, z)) = self.state.apply(axis, value as f32 / 32767.0) {
                        report(natives.axis(self.id, code, x, y, z));
                    }
                }
            }
            _ => {}
        }
    }
}

fn report(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("rusty-blox: controller input failed: {error}");
    }
}

#[derive(Default)]
struct Layout {
    buttons: Vec<Option<i32>>,
    axes: Vec<Option<i32>>,
}

fn read_layout(node: i32) -> Layout {
    let caps = format!("/sys/class/input/js{node}/device/capabilities");
    let Ok(keys) = fs::read_to_string(format!("{caps}/key")) else {
        return Layout::standard();
    };
    let Ok(abs) = fs::read_to_string(format!("{caps}/abs")) else {
        return Layout::standard();
    };
    let key_codes = set_bits(&keys);
    let buttons: Vec<_> = key_codes
        .iter()
        .copied()
        .filter(|code| *code >= BTN_JOYSTICK)
        .chain(
            key_codes
                .iter()
                .copied()
                .filter(|code| (BTN_MISC..BTN_JOYSTICK).contains(code)),
        )
        .map(button_code)
        .collect();
    let abs_codes = set_bits(&abs);
    let right_stick_on_z = !abs_codes.contains(&ABS_RX) && !abs_codes.contains(&ABS_RY);
    let legacy_dualshock4 = read_device_id(node).is_some_and(|(vendor, product, version)| {
        vendor == 0x054c && matches!(product, 0x05c4 | 0x09cc) && version & 0x8000 == 0
    });
    let axes = abs_codes
        .iter()
        .copied()
        .map(|code| match (code, legacy_dualshock4) {
            (ABS_Z, true) => Some(AXIS_Z),
            (ABS_RZ, true) => Some(AXIS_RZ),
            (ABS_RX, true) => Some(AXIS_LTRIGGER),
            (ABS_RY, true) => Some(AXIS_RTRIGGER),
            (ABS_X, _) => Some(AXIS_X),
            (ABS_Y, _) => Some(AXIS_Y),
            (ABS_Z, false) if right_stick_on_z => Some(AXIS_Z),
            (ABS_Z, false) => Some(AXIS_LTRIGGER),
            (ABS_RX, _) => Some(AXIS_Z),
            (ABS_RY, _) => Some(AXIS_RZ),
            (ABS_RZ, false) if right_stick_on_z => Some(AXIS_RZ),
            (ABS_RZ, false) => Some(AXIS_RTRIGGER),
            (ABS_HAT0X, _) => Some(AXIS_HAT_X),
            (ABS_HAT0Y, _) => Some(AXIS_HAT_Y),
            _ => None,
        })
        .collect();
    Layout { buttons, axes }
}

fn read_device_id(node: i32) -> Option<(u16, u16, u16)> {
    let read = |field: &str| {
        let value =
            fs::read_to_string(format!("/sys/class/input/js{node}/device/id/{field}")).ok()?;
        u16::from_str_radix(value.trim(), 16).ok()
    };
    Some((read("vendor")?, read("product")?, read("version")?))
}

impl Layout {
    fn standard() -> Self {
        Self {
            buttons: (0..15).map(button_index).collect(),
            axes: (0..8).map(axis_index).collect(),
        }
    }
}

fn button_code(code: usize) -> Option<i32> {
    Some(match code {
        0x130 => 96,
        0x131 => 97,
        0x132 => 98,
        0x133 => 99,
        0x134 => 100,
        0x135 => 101,
        0x136 => 102,
        0x137 => 103,
        0x138 => 104,
        0x139 => 105,
        0x13a => 109,
        0x13b => 108,
        0x13c => 110,
        0x13d => 106,
        0x13e => 107,
        0x220 => 19,
        0x221 => 20,
        0x222 => 21,
        0x223 => 22,
        _ => return None,
    })
}

fn button_index(index: u8) -> Option<i32> {
    const CODES: [i32; 15] = [
        96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 109, 108, 110, 106, 107,
    ];
    CODES.get(index as usize).copied()
}

fn axis_index(index: u8) -> Option<i32> {
    const CODES: [i32; 8] = [
        AXIS_X,
        AXIS_Y,
        AXIS_LTRIGGER,
        AXIS_Z,
        AXIS_RZ,
        AXIS_RTRIGGER,
        AXIS_HAT_X,
        AXIS_HAT_Y,
    ];
    CODES.get(index as usize).copied()
}

fn set_bits(mask: &str) -> Vec<usize> {
    let words: Vec<_> = mask.split_whitespace().collect();
    let mut bits = Vec::new();
    for (word_index, word) in words.iter().rev().enumerate() {
        let Ok(word) = u64::from_str_radix(word, 16) else {
            continue;
        };
        for bit in 0..64 {
            if word & (1 << bit) != 0 {
                bits.push(word_index * 64 + bit);
            }
        }
    }
    bits
}

#[derive(Default)]
struct AxisState {
    left: (f32, f32),
    right: (f32, f32),
    hat: (f32, f32),
}

impl AxisState {
    fn apply(&mut self, axis: i32, value: f32) -> Option<(i32, f32, f32, f32)> {
        let value = value.clamp(-1.0, 1.0);
        match axis {
            AXIS_X => self.left.0 = value,
            AXIS_Y => self.left.1 = value,
            AXIS_Z => self.right.0 = value,
            AXIS_RZ => self.right.1 = value,
            AXIS_HAT_X => self.hat.0 = value,
            AXIS_HAT_Y => self.hat.1 = value,
            AXIS_LTRIGGER | AXIS_RTRIGGER => return Some((axis, 0.0, 0.0, (value + 1.0) / 2.0)),
            _ => return None,
        }
        Some(match axis {
            AXIS_X | AXIS_Y => (AXIS_X, self.left.0, -self.left.1, 0.0),
            AXIS_Z | AXIS_RZ => (AXIS_Z, self.right.0, -self.right.1, 0.0),
            AXIS_HAT_X => (axis, self.hat.0, self.hat.1, self.hat.0),
            _ => (axis, self.hat.0, self.hat.1, -self.hat.1),
        })
    }
}

fn decode(packet: &[u8; EVENT_SIZE]) -> (u8, u8, i16) {
    (
        packet[6],
        packet[7],
        i16::from_le_bytes([packet[4], packet[5]]),
    )
}

fn js_node(name: &str) -> Option<i32> {
    let digits = name.strip_prefix("js")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn open_pad(node: i32) -> Option<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(NONBLOCK)
        .open(format!("/dev/input/js{node}"))
        .ok()
}

fn device_name(node: i32) -> Option<String> {
    fs::read_to_string(format!("/sys/class/input/js{node}/device/name"))
        .ok()
        .map(|name| name.trim().to_owned())
}

fn is_gamepad(node: i32) -> bool {
    let Ok(mask) = fs::read_to_string(format!("/sys/class/input/js{node}/device/capabilities/key"))
    else {
        return true;
    };
    set_bits(&mask)
        .into_iter()
        .any(|code| (BTN_JOYSTICK..=BTN_THUMBR).contains(&code))
}
