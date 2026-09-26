//! The `Emulation` domain: device metrics, UA, locale, media and input hints.
//!
//! These are advisory engine hints. The crate validates the CDP shapes and
//! normalizes them into [`crate::host`] types; a driver that does not implement
//! one still returns success, because an emulation hint the engine cannot honor
//! must not fail the CDP command.

use crate::host::{DeviceMetrics, MediaFeature};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles an `Emulation.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Emulation.setDeviceMetricsOverride" => {
            let metrics = device_metrics(request)?;
            session
                .dev
                .set_device_metrics_override(&metrics)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.clearDeviceMetricsOverride" => {
            session
                .dev
                .clear_device_metrics_override()
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setUserAgentOverride" => {
            let user_agent = request.string_param("userAgent")?;
            session
                .dev
                .set_user_agent_override(
                    user_agent,
                    request.optional_string("acceptLanguage")?,
                    request.optional_string("platform")?,
                )
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setGeolocationOverride" => {
            let latitude = request.optional_f64("latitude")?;
            let longitude = request.optional_f64("longitude")?;
            let position = match (latitude, longitude) {
                (Some(lat), Some(lon)) => {
                    Some((lat, lon, request.optional_f64("accuracy")?.unwrap_or(0.0)))
                }
                (None, None) => None,
                _ => {
                    return Err(CdpError::invalid_params(
                        "latitude and longitude must be set together",
                    ));
                }
            };
            session
                .dev
                .set_geolocation_override(position)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.clearGeolocationOverride" => {
            session
                .dev
                .set_geolocation_override(None)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setTimezoneOverride" => {
            let timezone = request.string_param("timezoneId")?;
            session
                .dev
                .set_timezone_override(timezone)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setLocaleOverride" => {
            let locale = request.optional_string("locale")?;
            session
                .dev
                .set_locale_override(locale)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setEmulatedMedia" => {
            let media = request.optional_string("media")?;
            let features = media_features(request)?;
            session
                .dev
                .set_emulated_media(media, &features)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setScrollbarsHidden" => {
            let hidden = request.bool_param("hidden")?;
            session
                .dev
                .set_scrollbars_hidden(hidden)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setTouchEmulationEnabled" => {
            let enabled = request.bool_param("enabled")?;
            let max = request
                .optional_u64("maxTouchPoints")?
                .map(|value| value.min(u32::MAX as u64) as u32);
            session
                .dev
                .set_touch_emulation_enabled(enabled, max)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setCPUThrottlingRate" => {
            let rate = request.number_param("rate")?;
            session
                .dev
                .set_cpu_throttling_rate(rate)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Emulation.setScriptExecutionDisabled" => {
            let value = request.bool_param("value")?;
            session
                .dev
                .set_script_execution_disabled(value)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

fn device_metrics(request: &CdpRequest) -> Result<DeviceMetrics, CdpError> {
    let (orientation_type, orientation_angle) = match request.params.get("screenOrientation") {
        None | Some(Json::Null) => (None, None),
        Some(value) => (
            value.get("type").and_then(Json::as_str).map(str::to_string),
            value.get("angle").and_then(Json::as_i64),
        ),
    };
    Ok(DeviceMetrics {
        width: request.u64_param("width")?.min(u32::MAX as u64) as u32,
        height: request.u64_param("height")?.min(u32::MAX as u64) as u32,
        device_scale_factor: request.number_param("deviceScaleFactor")?,
        mobile: request.bool_param("mobile")?,
        screen_width: request
            .optional_u64("screenWidth")?
            .map(|value| value.min(u32::MAX as u64) as u32),
        screen_height: request
            .optional_u64("screenHeight")?
            .map(|value| value.min(u32::MAX as u64) as u32),
        position_x: request.optional_i64("positionX")?,
        position_y: request.optional_i64("positionY")?,
        dont_set_visible_size: request.optional_bool("dontSetVisibleSize", false)?,
        orientation_type,
        orientation_angle,
    })
}

fn media_features(request: &CdpRequest) -> Result<Vec<MediaFeature>, CdpError> {
    let Some(raw) = request.params.get("features") else {
        return Ok(Vec::new());
    };
    if raw == &Json::Null {
        return Ok(Vec::new());
    }
    let raw = raw
        .as_array()
        .ok_or_else(|| CdpError::invalid_params("features must be an array"))?;
    let mut features = Vec::with_capacity(raw.len());
    for item in raw {
        let name = item
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::invalid_params("features[].name must be a string"))?;
        let value = item
            .get("value")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::invalid_params("features[].value must be a string"))?;
        features.push(MediaFeature {
            name: name.to_string(),
            value: value.to_string(),
        });
    }
    Ok(features)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};
    use crate::session::SessionState;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recording {
        metrics: Option<DeviceMetrics>,
        user_agent: Option<String>,
        geolocation: Option<Option<(f64, f64, f64)>>,
        media: Option<(Option<String>, Vec<MediaFeature>)>,
        touch: Option<(bool, Option<u32>)>,
        cpu: Option<f64>,
    }

    struct Emulator(Arc<Mutex<Recording>>);

    impl DevToolsSession for Emulator {
        fn set_device_metrics_override(&mut self, metrics: &DeviceMetrics) -> Result<(), HostError> {
            self.0.lock().unwrap().metrics = Some(metrics.clone());
            Ok(())
        }
        fn set_user_agent_override(
            &mut self,
            user_agent: &str,
            _accept_language: Option<&str>,
            _platform: Option<&str>,
        ) -> Result<(), HostError> {
            self.0.lock().unwrap().user_agent = Some(user_agent.to_string());
            Ok(())
        }
        fn set_geolocation_override(
            &mut self,
            position: Option<(f64, f64, f64)>,
        ) -> Result<(), HostError> {
            self.0.lock().unwrap().geolocation = Some(position);
            Ok(())
        }
        fn set_emulated_media(
            &mut self,
            media: Option<&str>,
            features: &[MediaFeature],
        ) -> Result<(), HostError> {
            self.0.lock().unwrap().media = Some((media.map(str::to_string), features.to_vec()));
            Ok(())
        }
        fn set_touch_emulation_enabled(
            &mut self,
            enabled: bool,
            max_touch_points: Option<u32>,
        ) -> Result<(), HostError> {
            self.0.lock().unwrap().touch = Some((enabled, max_touch_points));
            Ok(())
        }
        fn set_cpu_throttling_rate(&mut self, rate: f64) -> Result<(), HostError> {
            self.0.lock().unwrap().cpu = Some(rate);
            Ok(())
        }
    }

    fn session() -> (SessionState, Arc<Mutex<Recording>>) {
        let recording = Arc::new(Mutex::new(Recording::default()));
        let session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", ""),
            Box::new(Emulator(Arc::clone(&recording))),
        );
        (session, recording)
    }

    #[test]
    fn applies_and_normalizes_emulation_hints() {
        let (mut session, recording) = session();

        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Emulation.setDeviceMetricsOverride","params":{"width":390,"height":844,"deviceScaleFactor":3,"mobile":true,"screenOrientation":{"type":"portraitPrimary","angle":0}}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":2,"method":"Emulation.setUserAgentOverride","params":{"userAgent":"Amazing/Mobile"}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":3,"method":"Emulation.setEmulatedMedia","params":{"media":"print","features":[{"name":"prefers-color-scheme","value":"dark"}]}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":4,"method":"Emulation.setTouchEmulationEnabled","params":{"enabled":true,"maxTouchPoints":5}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":5,"method":"Emulation.setGeolocationOverride","params":{"latitude":1.5,"longitude":2.5}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":6,"method":"Emulation.setCPUThrottlingRate","params":{"rate":4}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let recording = recording.lock().unwrap();
        let metrics = recording.metrics.as_ref().unwrap();
        assert_eq!((metrics.width, metrics.height), (390, 844));
        assert!(metrics.mobile);
        assert_eq!(metrics.orientation_type.as_deref(), Some("portraitPrimary"));
        assert_eq!(recording.user_agent.as_deref(), Some("Amazing/Mobile"));
        let (media, features) = recording.media.as_ref().unwrap();
        assert_eq!(media.as_deref(), Some("print"));
        assert_eq!(features[0].name, "prefers-color-scheme");
        assert_eq!(recording.touch, Some((true, Some(5))));
        assert_eq!(recording.geolocation, Some(Some((1.5, 2.5, 0.0))));
        assert_eq!(recording.cpu, Some(4.0));
    }

    #[test]
    fn missing_mobile_flag_is_invalid_params() {
        let (mut session, _) = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Emulation.setDeviceMetricsOverride","params":{"width":1,"height":1,"deviceScaleFactor":1}}"#,
        )
        .unwrap();
        assert_eq!(
            dispatch(&mut session, &request).unwrap_err().code,
            crate::protocol::INVALID_PARAMS
        );
    }
}
