//! Immutable native CSP Reporting dictionary snapshot. No DOM/JS dependency.
use super::{CspDisposition, CspInlineViolation};

#[derive(Clone, Debug, PartialEq)]
pub enum CspReportValue {
    Null,
    Number(f64),
    String(Vec<u16>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CspReportVisibility {
    ObserverVisible,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CspReportSnapshot {
    pub report_type: Vec<u16>,
    /// Saved creation URL from the actual native violation, never current Location.
    pub url: Vec<u16>,
    pub visibility: CspReportVisibility,
    pub body: Vec<(Vec<u16>, CspReportValue)>,
}
impl CspInlineViolation {
    /// Registered csp-violation is observer-visible independent of endpoint
    /// existence, delivery admission, CORS success or HTTP status.
    /// HTML maps this native dictionary directly into ReportingValue::Object.
    pub fn report_snapshot(&self) -> CspReportSnapshot {
        fn string(value: &str) -> CspReportValue {
            CspReportValue::String(value.encode_utf16().collect())
        }
        let source_present = !self.source_file.is_empty();
        let fields = vec![
            ("documentURL", string(&self.document_uri)),
            ("referrer", string(&self.referrer)),
            ("blockedURL", string(&self.blocked_uri)),
            ("effectiveDirective", string(&self.effective_directive)),
            (
                "originalPolicy",
                CspReportValue::String(self.original_policy_units.clone()),
            ),
            (
                "sourceFile",
                if source_present {
                    string(&self.source_file)
                } else {
                    CspReportValue::Null
                },
            ),
            ("sample", CspReportValue::String(self.sample.clone())),
            (
                "disposition",
                string(match self.disposition {
                    CspDisposition::Enforce => "enforce",
                    CspDisposition::ReportOnly => "report",
                }),
            ),
            (
                "statusCode",
                CspReportValue::Number(f64::from(self.status_code)),
            ),
            (
                "lineNumber",
                if source_present {
                    CspReportValue::Number(f64::from(self.line_number))
                } else {
                    CspReportValue::Null
                },
            ),
            (
                "columnNumber",
                if source_present {
                    CspReportValue::Number(f64::from(self.column_number))
                } else {
                    CspReportValue::Null
                },
            ),
        ];
        CspReportSnapshot {
            report_type: "csp-violation".encode_utf16().collect(),
            url: self.report_url.encode_utf16().collect(),
            visibility: CspReportVisibility::ObserverVisible,
            body: fields
                .into_iter()
                .map(|(key, value)| (key.encode_utf16().collect(), value))
                .collect(),
        }
    }
}
