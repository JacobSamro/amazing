//! The `Page` domain: navigation, reload, frame tree, lifecycle and screenshots.

use super::{frame, frame_tree, object};
use crate::base64;
use crate::host::{FrameInfo, LifecycleEvent, Rect, ScreenshotFormat, ScreenshotParams};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Page.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Page.enable" => {
            session.enabled.page = true;
            let mut events = Vec::new();
            if let Ok(tree) = session.dev.frame_tree() {
                events.push(frame_navigated(&tree.frame, "Navigation"));
                events.push(lifecycle_event(&LifecycleEvent {
                    frame_id: tree.frame.id.clone(),
                    name: "init".into(),
                    loader_id: String::new(),
                    timestamp: 0.0,
                }));
            }
            Ok(DispatchOutcome::empty().with_events(events))
        }
        "Page.disable" => {
            session.enabled.page = false;
            Ok(DispatchOutcome::empty())
        }
        "Page.navigate" => {
            let url = request.string_param("url")?;
            let info = session.dev.navigate(url).map_err(CdpError::from)?;
            let events = vec![frame_navigated(&info, "Navigation")];
            Ok(DispatchOutcome::result(object(vec![
                ("frameId", Json::String(info.id.clone())),
                ("loaderId", Json::String(String::new())),
            ]))
            .with_events(events))
        }
        "Page.reload" => {
            let ignore_cache = request.optional_bool("ignoreCache", false)?;
            session.dev.reload(ignore_cache).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Page.getFrameTree" => {
            let tree = session.dev.frame_tree().map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "frameTree",
                frame_tree(&tree),
            )])))
        }
        "Page.captureScreenshot" => {
            let params = screenshot_params(request)?;
            let image = session
                .dev
                .capture_screenshot(&params)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "data",
                Json::String(base64::encode(&image.data)),
            )])))
        }
        "Page.getNavigationHistory" => {
            let entry = object(vec![
                ("id", Json::number(1.0)),
                ("url", Json::String(session.target.url.clone())),
                ("userTypedURL", Json::String(session.target.url.clone())),
                ("title", Json::String(session.target.title.clone())),
                ("transitionType", Json::String("typed".into())),
            ]);
            Ok(DispatchOutcome::result(object(vec![
                ("currentIndex", Json::number(0.0)),
                ("entries", Json::Array(vec![entry])),
            ])))
        }
        "Page.setLifecycleEventsEnabled" => {
            request.optional_bool("enabled", true)?;
            Ok(DispatchOutcome::empty())
        }
        "Page.bringToFront" => Ok(DispatchOutcome::empty()),
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Builds the `Page.frameNavigated` event.
pub fn frame_navigated(info: &FrameInfo, kind: &str) -> Json {
    crate::protocol::event(
        "Page.frameNavigated",
        object(vec![
            ("frame", frame(info)),
            ("type", Json::String(kind.to_string())),
        ]),
    )
}

/// Builds the `Page.lifecycleEvent` event.
pub fn lifecycle_event(item: &LifecycleEvent) -> Json {
    crate::protocol::event(
        "Page.lifecycleEvent",
        object(vec![
            ("frameId", Json::String(item.frame_id.clone())),
            ("loaderId", Json::String(item.loader_id.clone())),
            ("name", Json::String(item.name.clone())),
            ("timestamp", Json::number(item.timestamp)),
        ]),
    )
}

/// Builds the `Page.loadEventFired` event from a monotonic timestamp.
pub fn load_event(timestamp: f64) -> Json {
    crate::protocol::event(
        "Page.loadEventFired",
        object(vec![("timestamp", Json::number(timestamp))]),
    )
}

fn screenshot_params(request: &CdpRequest) -> Result<ScreenshotParams, CdpError> {
    let format = match request.optional_string("format")? {
        None | Some("png") => ScreenshotFormat::Png,
        Some("jpeg") => ScreenshotFormat::Jpeg,
        Some("webp") => ScreenshotFormat::Webp,
        Some(other) => {
            return Err(CdpError::invalid_params(format!(
                "unsupported screenshot format '{other}'"
            )));
        }
    };
    let quality = request
        .optional_u64("quality")?
        .map(|q| q.min(100) as u8);
    let clip = match request.params.get("clip") {
        None | Some(Json::Null) => None,
        Some(value) => Some(parse_clip(value)?),
    };
    Ok(ScreenshotParams {
        format,
        quality,
        clip,
        capture_beyond_viewport: request.optional_bool("captureBeyondViewport", false)?,
        from_surface: request.optional_bool("fromSurface", true)?,
    })
}

fn parse_clip(value: &Json) -> Result<Rect, CdpError> {
    let number = |name: &str| -> Result<f64, CdpError> {
        value
            .get(name)
            .and_then(Json::as_f64)
            .ok_or_else(|| CdpError::invalid_params(format!("clip.{name} must be a number")))
    };
    Ok(Rect {
        x: number("x")?,
        y: number("y")?,
        width: number("width")?,
        height: number("height")?,
        scale: match value.get("scale") {
            None | Some(Json::Null) => 1.0,
            Some(scale) => scale
                .as_f64()
                .ok_or_else(|| CdpError::invalid_params("clip.scale must be a number"))?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, FrameInfo, FrameTree, HostError};
    use crate::session::SessionState;
    use std::sync::{Arc, Mutex};

    struct Fake {
        navigated: Arc<Mutex<Vec<String>>>,
        tree: Option<FrameTree>,
    }

    impl DevToolsSession for Fake {
        fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
            self.navigated.lock().unwrap().push(url.to_string());
            Ok(FrameInfo {
                id: "F1".into(),
                parent_id: None,
                url: url.to_string(),
                name: String::new(),
                security_origin: "https://a".into(),
                mime_type: "text/html".into(),
            })
        }
        fn frame_tree(&mut self) -> Result<FrameTree, HostError> {
            self.tree
                .clone()
                .ok_or(HostError::Unsupported)
        }
    }

    fn session(dev: Fake) -> SessionState {
        SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "https://a/", "A"),
            Box::new(dev),
        )
    }

    #[test]
    fn navigate_returns_frame_and_emits_frame_navigated() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut session = session(Fake {
            navigated: seen.clone(),
            tree: None,
        });
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"Page.navigate","params":{"url":"https://b/"}}"#)
                .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(outcome.result.get("frameId").unwrap().as_str(), Some("F1"));
        assert_eq!(seen.lock().unwrap().as_slice(), ["https://b/"]);
        assert_eq!(
            outcome.events[0].get("method").unwrap().as_str(),
            Some("Page.frameNavigated")
        );
    }

    #[test]
    fn enable_emits_existing_frame_and_init() {
        let tree = FrameTree {
            frame: FrameInfo {
                id: "F1".into(),
                parent_id: None,
                url: "https://a/".into(),
                name: String::new(),
                security_origin: "https://a".into(),
                mime_type: "text/html".into(),
            },
            child_frames: Vec::new(),
        };
        let mut session = session(Fake {
            navigated: Arc::new(Mutex::new(Vec::new())),
            tree: Some(tree),
        });
        let request = CdpRequest::parse(r#"{"id":1,"method":"Page.enable"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert!(session.enabled.page);
        assert_eq!(outcome.events.len(), 2);
        assert_eq!(
            outcome.events[1].get("params").unwrap().get("name").unwrap().as_str(),
            Some("init")
        );
    }

    #[test]
    fn screenshot_is_base64_encoded() {
        struct Shot;
        impl DevToolsSession for Shot {
            fn capture_screenshot(
                &mut self,
                _params: &ScreenshotParams,
            ) -> Result<crate::host::Screenshot, HostError> {
                Ok(crate::host::Screenshot {
                    data: b"png".to_vec(),
                    format: ScreenshotFormat::Png,
                })
            }
        }
        let mut session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", ""),
            Box::new(Shot),
        );
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"Page.captureScreenshot","params":{"format":"png"}}"#)
                .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(outcome.result.get("data").unwrap().as_str(), Some("cG5n"));
    }
}
