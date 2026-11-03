use super::*;
use wire::*;
fn limits() -> Limits {
    Limits { max_message_bytes: 8 * 1024 * 1024, max_texture_pixels: 4 * 1024 * 1024, max_vertices: 1_000_000, max_events: 256 }
}
fn input() -> PanelInput {
    PanelInput {
        size: [380.0, 600.0],
        pixels_per_point: 2.0,
        time: 1.0,
        focused: true,
        dark: true,
        palette: None,
        modifiers: Modifiers::default(),
        events: vec![],
    }
}
#[test]
fn bounded_pipe_roundtrip_and_truncation() {
    let message = HostMessage::Command { id: "1".into(), name: "status".into(), args: serde_json::json!({}) };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message, 4096).unwrap();
    assert!(matches!(read_message::<HostMessage>(&mut bytes.as_slice(), 4096).unwrap(), Some(HostMessage::Command { .. })));
    assert!(read_message::<HostMessage>(&mut bytes.as_slice(), 1).is_err());
    bytes.pop();
    assert!(read_message::<HostMessage>(&mut bytes.as_slice(), 4096).is_err());
    assert!(read_message::<HostMessage>(&mut &[][..], 4096).unwrap().is_none());
}
#[test]
fn malformed_mesh_and_texture_rejected_before_paint() {
    let mut frame = PanelFrame::default();
    frame.meshes.push(Mesh { clip: [0.0; 4], texture: 0, vertices: vec![], indices: vec![0, 0, 0] });
    assert!(frame.validate(&limits()).is_err());
    frame.meshes.clear();
    frame.textures.push(Texture {
        id: 0,
        size: [usize::MAX, usize::MAX],
        offset: None,
        rgba: vec![],
        magnify_linear: true,
        minify_linear: true,
        wrap: 0,
    });
    assert!(frame.validate(&limits()).is_err());
    let mut bad = input();
    bad.pixels_per_point = f32::NAN;
    assert!(bad.validate(&limits()).is_err());
}
#[test]
fn helper_renders_real_text_edit_with_retina_textures_and_accessibility() {
    let mut renderer = PanelRenderer::new(limits()).unwrap();
    let mut text = String::new();
    let id = egui::Id::unique("test-remote-editor");
    let frame = renderer
        .frame(input(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(id));
            ui.ctx().memory_mut(|m| m.request_focus(id));
        })
        .unwrap();
    assert!(!frame.meshes.is_empty());
    assert!(!frame.textures.is_empty());
    assert_eq!(frame.accessibility.as_ref().unwrap().schema, ACCESSIBILITY_SCHEMA);
    let mut next = input();
    next.time = 2.0;
    next.events.push(InputEvent::Text { text: "hello".into() });
    renderer
        .frame(next, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(id));
        })
        .unwrap();
    assert_eq!(text, "hello");
    let mut next = input();
    next.time = 3.0;
    next.events.push(InputEvent::ImeCommit { text: "日本語".into() });
    let frame = renderer
        .frame(next, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(id));
        })
        .unwrap();
    assert_eq!(text, "hello日本語");
    assert!(frame.ime.is_some());
}
#[test]
fn explicit_ime_byte_delete_and_scroll_preserve_units() {
    let mut i = input();
    i.events = vec![
        InputEvent::ImeDeleteBytes { before: 4, after: 0 },
        InputEvent::Wheel { delta: [0.0, 2.0], unit: 1, phase: 2, source: 3, modifiers: Modifiers::default() },
    ];
    let raw = i.into_raw().unwrap();
    assert!(matches!(raw.events.get(1), Some(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 4, after_bytes: 0 }))));
    assert!(matches!(
        raw.events.get(2),
        Some(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            phase: egui::TouchPhase::End,
            source: egui::MouseWheelSource::Momentum,
            ..
        })
    ));
}

#[test]
#[cfg(unix)]
fn terminated_helper_is_reported_without_restarting_or_replaying() {
    let mut client = Client::spawn(std::path::Path::new("/usr/bin/false"), limits()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while client.error().is_none() && std::time::Instant::now() < deadline {
        client.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(client.error().is_some());
    assert!(client.send(HostMessage::Command { id: "uncertain".into(), name: "send".into(), args: serde_json::json!({}) }).is_err());
}

#[test]
fn modifier_snapshot_enters_helper_as_an_explicit_event() {
    let mut i = input();
    i.modifiers.shift = true;
    assert!(matches!(i.into_raw().unwrap().events.first(),Some(egui::Event::ModifiersChanged(m)) if m.shift));
}

#[test]
fn malformed_ime_state_is_rejected_before_native_output() {
    let mut frame = PanelFrame {
        ime: Some(Ime {
            owner: 1,
            generation: 1,
            purpose: 3,
            rect: [0.0, 0.0, 10.0, 10.0],
            cursor: [0.0, 0.0, 1.0, 10.0],
            interrupt: false,
            surrounding: None,
            input_method_change: false,
            reset_serial: None,
        }),
        ..PanelFrame::default()
    };
    assert!(frame.validate(&limits()).is_err());
    let ime = frame.ime.as_mut().unwrap();
    ime.purpose = 0;
    ime.surrounding = Some(Surrounding { text: "é".into(), cursor: 1, anchor: 0 });
    assert!(frame.validate(&limits()).is_err());
}
