//! Protocol 1. Rendering uses triangles and premultiplied RGBA, not egui serialization.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};

pub const PROTOCOL: u32 = 1;
/// AccessKit's separately versioned public tree schema, negotiated independently of egui.
pub const ACCESSIBILITY_SCHEMA: &str = "accesskit-0.25";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_message_bytes: usize,
    pub max_texture_pixels: usize,
    pub max_vertices: usize,
    pub max_events: usize,
}
impl Limits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_message_bytes == 0
            || self.max_message_bytes > 64 * 1024 * 1024
            || self.max_texture_pixels == 0
            || self.max_texture_pixels > 64 * 1024 * 1024
            || self.max_vertices == 0
            || self.max_vertices > 2_000_000
            || self.max_events == 0
            || self.max_events > 4096
        {
            return Err("assistant bridge limits are outside supported positive safety bounds".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Initialize { protocol: u32, app_id: String, config: Value },
    Frame { input: PanelInput },
    Command { id: String, name: String, args: Value },
    ToolResult { id: String, result: Value },
    Shutdown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperMessage {
    Ready { protocol: u32, version: String, capabilities: Vec<String> },
    Frame(Box<PanelFrame>),
    Event(Value),
    ToolCall { id: String, payload: Value },
    Error { message: String },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub mac_cmd: bool,
    pub command: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    Text {
        text: String,
    },
    Paste {
        text: String,
    },
    Copy,
    Cut,
    Key {
        key: String,
        physical: Option<String>,
        pressed: bool,
        repeat: bool,
        modifiers: Modifiers,
    },
    Modifiers {
        modifiers: Modifiers,
    },
    PointerMoved {
        position: [f32; 2],
    },
    PointerGone,
    PointerButton {
        position: [f32; 2],
        button: u8,
        pressed: bool,
        modifiers: Modifiers,
    },
    Wheel {
        delta: [f32; 2],
        unit: u8,
        phase: u8,
        source: u8,
        modifiers: Modifiers,
    },
    Zoom {
        factor: f32,
    },
    Rotate {
        radians: f32,
    },
    ImePreedit {
        text: String,
        active_chars: Option<[usize; 2]>,
    },
    ImeCommit {
        text: String,
    },
    ImeResetCompleted {
        serial: u64,
    },
    ImeDeleteChars {
        before: usize,
        after: usize,
    },
    ImeDeleteBytes {
        before: usize,
        after: usize,
    },
    WindowFocused {
        focused: bool,
    },
    /// Public AccessKit action schema 0.25, independent of the egui version.
    Accessibility {
        action: Value,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Palette {
    pub text: [u8; 4],
    pub panel: [u8; 4],
    pub window: [u8; 4],
    pub selection: [u8; 4],
    pub hyperlink: [u8; 4],
    pub extreme_background: [u8; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PanelInput {
    pub size: [f32; 2],
    pub pixels_per_point: f32,
    pub time: f64,
    pub focused: bool,
    pub dark: bool,
    pub modifiers: Modifiers,
    #[serde(default)]
    pub palette: Option<Palette>,
    pub events: Vec<InputEvent>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mesh {
    pub clip: [f32; 4],
    pub texture: u64,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Texture {
    pub id: u64,
    pub size: [usize; 2],
    pub offset: Option<[usize; 2]>,
    pub rgba: Vec<u8>,
    pub magnify_linear: bool,
    pub minify_linear: bool,
    pub wrap: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ime {
    pub owner: u64,
    pub generation: u64,
    pub purpose: u8,
    pub rect: [f32; 4],
    pub cursor: [f32; 4],
    pub interrupt: bool,
    pub surrounding: Option<Surrounding>,
    pub input_method_change: bool,
    pub reset_serial: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Surrounding {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformCommand {
    CopyText { text: String },
    Selection { text: String },
    OpenUrl { url: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Accessibility {
    pub schema: String,
    pub root: u64,
    pub focus: u64,
    pub nodes: Vec<(u64, Value)>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PanelFrame {
    /// true leaves the panel forward, false leaves it backward.
    #[serde(default)]
    pub focus_exit: Option<bool>,
    pub meshes: Vec<Mesh>,
    pub textures: Vec<Texture>,
    pub free_textures: Vec<u64>,
    pub cursor: String,
    pub ime: Option<Ime>,
    pub commands: Vec<PlatformCommand>,
    pub accessibility: Option<Accessibility>,
    pub wants_keyboard: bool,
    pub wants_pointer: bool,
}
impl PanelInput {
    pub fn validate(&self, limits: &Limits) -> Result<(), String> {
        if !self.size.into_iter().all(|v| v.is_finite() && v > 0.0 && v <= 32768.0)
            || !self.pixels_per_point.is_finite()
            || !(0.25..=16.0).contains(&self.pixels_per_point)
            || !self.time.is_finite()
            || self.events.len() > limits.max_events
        {
            return Err("invalid assistant panel dimensions, scale, time, or event count".into());
        }
        for event in &self.events {
            match event {
                InputEvent::PointerMoved { position } | InputEvent::PointerButton { position, .. } if !position.iter().all(|v| v.is_finite()) => {
                    return Err("non-finite assistant pointer".into());
                }
                InputEvent::Wheel { delta, .. } if !delta.iter().all(|v| v.is_finite()) => return Err("non-finite assistant wheel".into()),
                InputEvent::Zoom { factor } if !factor.is_finite() || *factor <= 0.0 => return Err("invalid assistant zoom".into()),
                InputEvent::Rotate { radians } if !radians.is_finite() => return Err("invalid assistant rotation".into()),
                InputEvent::ImePreedit { text, active_chars: Some(range) } if range[0] > range[1] || range[1] > text.chars().count() => {
                    return Err("invalid assistant preedit range".into());
                }
                _ => {}
            }
        }
        Ok(())
    }
}
impl PanelFrame {
    pub fn validate(&self, limits: &Limits) -> Result<(), String> {
        if self.meshes.len() > 4096 || self.textures.len() > 4096 || self.free_textures.len() > 4096 {
            return Err("assistant frame has too many mesh or texture records".into());
        }
        let mut vertices = 0usize;
        let mut pixels = 0usize;
        let mut indices = 0usize;
        for mesh in &self.meshes {
            vertices = vertices.checked_add(mesh.vertices.len()).ok_or("assistant vertex count overflow")?;
            indices = indices.checked_add(mesh.indices.len()).ok_or("assistant index count overflow")?;
            if vertices > limits.max_vertices
                || indices > limits.max_vertices.saturating_mul(6)
                || mesh.indices.len() % 3 != 0
                || !mesh.clip.into_iter().all(f32::is_finite)
                || mesh.vertices.iter().any(|v| !v.position.into_iter().chain(v.uv).all(f32::is_finite))
                || mesh.indices.iter().any(|&i| i as usize >= mesh.vertices.len())
            {
                return Err("invalid or oversized assistant mesh".into());
            }
        }
        for texture in &self.textures {
            let count = texture.size[0].checked_mul(texture.size[1]).ok_or("assistant texture overflow")?;
            pixels = pixels.checked_add(count).ok_or("assistant texture total overflow")?;
            if texture.size.contains(&0)
                || texture.size.iter().any(|side| *side > 16384)
                || pixels > limits.max_texture_pixels
                || count.checked_mul(4) != Some(texture.rgba.len())
            {
                return Err("invalid or oversized assistant texture".into());
            }
        }
        if let Some(ime) = &self.ime {
            let valid_rect = |rect: [f32; 4]| rect.into_iter().all(f32::is_finite) && rect[0] <= rect[2] && rect[1] <= rect[3];
            if ime.purpose > 2 || ime.owner == 0 || !valid_rect(ime.rect) || !valid_rect(ime.cursor) || ime.reset_serial == Some(0) {
                return Err("invalid assistant IME state".into());
            }
            if let Some(s) = &ime.surrounding
                && (s.cursor > s.text.len() || s.anchor > s.text.len() || !s.text.is_char_boundary(s.cursor) || !s.text.is_char_boundary(s.anchor))
            {
                return Err("invalid assistant IME surrounding-text selection".into());
            }
        }
        if let Some(a) = &self.accessibility
            && (a.schema != ACCESSIBILITY_SCHEMA || a.nodes.len() > limits.max_vertices)
        {
            return Err("unsupported or oversized assistant accessibility tree".into());
        }
        Ok(())
    }
}
/// Read one bounded message. Clean EOF is distinct from a truncated message.
pub fn read_message<T: serde::de::DeserializeOwned>(reader: &mut impl Read, max: usize) -> Result<Option<T>, String> {
    let mut header = [0u8; 4];
    match reader.read(&mut header[..1]).map_err(|e| e.to_string())? {
        0 => return Ok(None),
        _ => reader.read_exact(&mut header[1..]).map_err(|e| e.to_string())?,
    }
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > max {
        return Err("assistant message exceeds configured byte limit".into());
    }
    let mut data = vec![0; len];
    reader.read_exact(&mut data).map_err(|e| e.to_string())?;
    serde_json::from_slice(&data).map(Some).map_err(|e| format!("invalid assistant message: {e}"))
}
pub fn write_message<T: Serialize>(writer: &mut impl Write, value: &T, max: usize) -> Result<(), String> {
    let data = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if data.len() > max {
        return Err("assistant message exceeds configured byte limit".into());
    }
    let len = u32::try_from(data.len()).map_err(|_| "assistant message exceeds framing limit")?;
    writer.write_all(&len.to_be_bytes()).and_then(|()| writer.write_all(&data)).and_then(|()| writer.flush()).map_err(|e| e.to_string())
}
