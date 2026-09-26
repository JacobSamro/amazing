//! The `Input` domain: synthesized mouse, keyboard, touch and text events.
//!
//! The crate does not interpret these events; it validates and normalizes the
//! CDP shapes and hands them to [`crate::host::DevToolsSession`], which routes
//! them into the engine's own input pipeline exactly as real input would be.

use crate::host::{KeyEvent, MouseEvent, TouchEvent, TouchPoint};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles an `Input.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Input.setIgnoreInputEvents" => {
            let ignore = request.optional_bool("ignore", false)?;
            session
                .dev
                .set_ignore_input_events(ignore)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Input.dispatchMouseEvent" => {
            let event = mouse_event(request)?;
            session
                .dev
                .dispatch_mouse_event(&event)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Input.dispatchKeyEvent" => {
            let event = key_event(request)?;
            session
                .dev
                .dispatch_key_event(&event)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Input.dispatchTouchEvent" => {
            let event = touch_event(request)?;
            session
                .dev
                .dispatch_touch_event(&event)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Input.insertText" => {
            let text = request.string_param("text")?;
            session.dev.insert_text(text).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

fn mouse_event(request: &CdpRequest) -> Result<MouseEvent, CdpError> {
    Ok(MouseEvent {
        kind: request.string_param("type")?.to_string(),
        x: request.number_param("x")?,
        y: request.number_param("y")?,
        modifiers: request.optional_u64("modifiers")?.unwrap_or(0) as u32,
        button: request
            .optional_string("button")?
            .unwrap_or("none")
            .to_string(),
        buttons: request.optional_u64("buttons")?.unwrap_or(0) as u32,
        click_count: request.optional_u64("clickCount")?.unwrap_or(0) as u32,
        delta_x: request.optional_f64("deltaX")?.unwrap_or(0.0),
        delta_y: request.optional_f64("deltaY")?.unwrap_or(0.0),
        timestamp: request.optional_f64("timestamp")?.unwrap_or(0.0),
    })
}

fn key_event(request: &CdpRequest) -> Result<KeyEvent, CdpError> {
    Ok(KeyEvent {
        kind: request.string_param("type")?.to_string(),
        modifiers: request.optional_u64("modifiers")?.unwrap_or(0) as u32,
        text: request.optional_string("text")?.map(str::to_string),
        unmodified_text: request.optional_string("unmodifiedText")?.map(str::to_string),
        key: request.optional_string("key")?.map(str::to_string),
        code: request.optional_string("code")?.map(str::to_string),
        windows_virtual_key_code: request.optional_i64("windowsVirtualKeyCode")?,
        native_virtual_key_code: request.optional_i64("nativeVirtualKeyCode")?,
        auto_repeat: request.optional_bool("autoRepeat", false)?,
        is_keypad: request.optional_bool("isKeypad", false)?,
        is_system_key: request.optional_bool("isSystemKey", false)?,
        location: request.optional_string("location")?.map(str::to_string),
    })
}

fn touch_event(request: &CdpRequest) -> Result<TouchEvent, CdpError> {
    let raw = request
        .params
        .get("touchPoints")
        .and_then(Json::as_array)
        .ok_or_else(|| CdpError::invalid_params("touchPoints must be an array"))?;
    let mut points = Vec::with_capacity(raw.len());
    for item in raw {
        points.push(TouchPoint {
            x: number(item, "x")?,
            y: number(item, "y")?,
            radius_x: optional_number(item, "radiusX")?.unwrap_or(1.0),
            radius_y: optional_number(item, "radiusY")?.unwrap_or(1.0),
            rotation_angle: optional_number(item, "rotationAngle")?.unwrap_or(0.0),
            force: optional_number(item, "force")?.unwrap_or(0.0),
            id: optional_number(item, "id")?.unwrap_or(0.0),
        });
    }
    Ok(TouchEvent {
        kind: request.string_param("type")?.to_string(),
        points,
        modifiers: request.optional_u64("modifiers")?.unwrap_or(0) as u32,
        timestamp: request.optional_f64("timestamp")?.unwrap_or(0.0),
    })
}

fn number(value: &Json, name: &str) -> Result<f64, CdpError> {
    optional_number(value, name)?
        .ok_or_else(|| CdpError::invalid_params(format!("touchPoints[].{name} is required")))
}

fn optional_number(value: &Json, name: &str) -> Result<Option<f64>, CdpError> {
    match value.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(n)) if n.is_finite() => Ok(Some(*n)),
        Some(_) => Err(CdpError::invalid_params(format!(
            "touchPoints[].{name} must be a number"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};
    use crate::session::SessionState;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recording {
        mouse: Vec<MouseEvent>,
        keys: Vec<KeyEvent>,
        touches: Vec<TouchEvent>,
        text: Vec<String>,
        ignore: Option<bool>,
    }

    struct Recorder(Arc<Mutex<Recording>>);

    impl DevToolsSession for Recorder {
        fn set_ignore_input_events(&mut self, ignore: bool) -> Result<(), HostError> {
            self.0.lock().unwrap().ignore = Some(ignore);
            Ok(())
        }
        fn dispatch_mouse_event(&mut self, event: &MouseEvent) -> Result<(), HostError> {
            self.0.lock().unwrap().mouse.push(event.clone());
            Ok(())
        }
        fn dispatch_key_event(&mut self, event: &KeyEvent) -> Result<(), HostError> {
            self.0.lock().unwrap().keys.push(event.clone());
            Ok(())
        }
        fn dispatch_touch_event(&mut self, event: &TouchEvent) -> Result<(), HostError> {
            self.0.lock().unwrap().touches.push(event.clone());
            Ok(())
        }
        fn insert_text(&mut self, text: &str) -> Result<(), HostError> {
            self.0.lock().unwrap().text.push(text.to_string());
            Ok(())
        }
    }

    fn session() -> (SessionState, Arc<Mutex<Recording>>) {
        let recording = Arc::new(Mutex::new(Recording::default()));
        let session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", ""),
            Box::new(Recorder(Arc::clone(&recording))),
        );
        (session, recording)
    }

    #[test]
    fn dispatches_mouse_key_touch_and_text() {
        let (mut session, recording) = session();

        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Input.dispatchMouseEvent","params":{"type":"mousePressed","x":10,"y":20,"button":"left","buttons":1,"clickCount":1,"modifiers":8}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":2,"method":"Input.dispatchKeyEvent","params":{"type":"keyDown","key":"a","code":"KeyA","windowsVirtualKeyCode":65,"text":"a"}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":3,"method":"Input.dispatchTouchEvent","params":{"type":"touchStart","touchPoints":[{"x":1,"y":2,"id":3}]}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request =
            CdpRequest::parse(r#"{"id":4,"method":"Input.insertText","params":{"text":"héllo"}}"#)
                .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":5,"method":"Input.setIgnoreInputEvents","params":{"ignore":true}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let recording = recording.lock().unwrap();
        assert_eq!(recording.mouse.len(), 1);
        assert_eq!(recording.mouse[0].x, 10.0);
        assert_eq!(recording.mouse[0].click_count, 1);
        assert_eq!(recording.mouse[0].modifiers, 8);
        assert_eq!(recording.keys[0].windows_virtual_key_code, Some(65));
        assert_eq!(recording.keys[0].text.as_deref(), Some("a"));
        assert_eq!(recording.touches[0].points.len(), 1);
        assert_eq!(recording.touches[0].points[0].id, 3.0);
        assert_eq!(recording.text, ["héllo"]);
        assert_eq!(recording.ignore, Some(true));
    }

    #[test]
    fn missing_mouse_coordinates_are_invalid_params() {
        let (mut session, _) = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Input.dispatchMouseEvent","params":{"type":"mouseMoved"}}"#,
        )
        .unwrap();
        assert_eq!(
            dispatch(&mut session, &request).unwrap_err().code,
            crate::protocol::INVALID_PARAMS
        );
    }
}
