//! The `Performance` domain: engine metric counters and gauges.

use super::object;
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Performance.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Performance.enable" | "Performance.disable" => Ok(DispatchOutcome::empty()),
        "Performance.setTimeDomain" => {
            request.string_param("timeDomain")?;
            Ok(DispatchOutcome::empty())
        }
        "Performance.getMetrics" => {
            let metrics = session
                .dev
                .performance_metrics()
                .map_err(CdpError::from)?;
            let metrics: Vec<Json> = metrics
                .iter()
                .map(|(name, value)| {
                    object(vec![
                        ("name", Json::String(name.clone())),
                        ("value", Json::number(*value)),
                    ])
                })
                .collect();
            Ok(DispatchOutcome::result(object(vec![(
                "metrics",
                Json::Array(metrics),
            )])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};
    use crate::session::SessionState;

    struct Metrics;

    impl DevToolsSession for Metrics {
        fn performance_metrics(&mut self) -> Result<Vec<(String, f64)>, HostError> {
            Ok(vec![("Frames".into(), 12.0), ("Nodes".into(), 340.0)])
        }
    }

    #[test]
    fn serializes_named_metrics() {
        let mut session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", ""),
            Box::new(Metrics),
        );
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"Performance.getMetrics"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let metrics = outcome.result.get("metrics").unwrap().as_array().unwrap();
        assert_eq!(metrics[0].get("name").unwrap().as_str(), Some("Frames"));
        assert_eq!(metrics[1].get("value").unwrap().as_f64(), Some(340.0));
    }
}
