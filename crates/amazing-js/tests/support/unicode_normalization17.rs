//! Original Amazing corpus adapter; input data is official Unicode 17,
//! distributed under Unicode-3.0 in ../../data/unicode17/LICENSE.txt.
use amazing_js::{Interpreter, interp::ExecutionBackend, value::JsValue};
use std::collections::BTreeSet;

pub const FORMS: [&str; 4] = ["NFC", "NFD", "NFKC", "NFKD"];
pub const EXPECTED_ROWS: usize = 20_034;
const DATA: &str = include_str!("../../data/unicode17/NormalizationTest.txt");

pub struct Case {
    pub line: usize,
    pub columns: [Vec<u16>; 5],
}

fn push_units(point: u32, units: &mut Vec<u16>) {
    assert!(point <= 0x10ffff);
    if point <= 0xffff {
        units.push(point as u16);
    } else {
        let point = point - 0x10000;
        units.extend([0xd800 + (point >> 10) as u16, 0xdc00 + (point & 0x3ff) as u16]);
    }
}

pub fn cases() -> Vec<Case> {
    let result: Vec<_> = DATA.lines().enumerate().filter_map(|(index, line)| {
        let body = line.split('#').next().unwrap().trim();
        if body.is_empty() || body.starts_with('@') { return None; }
        let columns: Vec<Vec<u16>> = body.split(';').take(5).map(|column| {
            let mut units = Vec::new();
            for value in column.split_whitespace() {
                push_units(u32::from_str_radix(value, 16).unwrap(), &mut units);
            }
            units
        }).collect();
        Some(Case { line: index + 1, columns: columns.try_into().unwrap() })
    }).collect();
    assert_eq!(result.len(), EXPECTED_ROWS);
    result
}

pub struct SuiteResult {
    pub checks: usize,
    pub failed_checks: usize,
    pub row_statuses: Vec<(usize, bool)>,
    pub diagnostics: Vec<String>,
}

/// All twenty invariants for every official row, through the installed builtin.
pub fn run_rows(i: &mut Interpreter, backend: ExecutionBackend) -> SuiteResult {
    let method = i.run_script_with_backend("String.prototype.normalize", backend).unwrap();
    let forms = FORMS.map(JsValue::str);
    let mut result = SuiteResult { checks: 0, failed_checks: 0,
        row_statuses: Vec::with_capacity(EXPECTED_ROWS), diagnostics: Vec::new() };
    for case in cases() {
        let mut passed = true;
        for (form_index, form) in forms.iter().enumerate() {
            for column in 0..5 {
                let expected_index = match form_index {
                    0 => if column < 3 { 1 } else { 3 },
                    1 => if column < 3 { 2 } else { 4 },
                    2 => 3,
                    _ => 4,
                };
                let expected = JsValue::str_units(case.columns[expected_index].clone());
                let observed = i.binding_call(&method,
                    JsValue::str_units(case.columns[column].clone()), vec![form.clone()]);
                result.checks += 1;
                let matches = observed.as_ref().is_ok_and(|value| value == &expected);
                if !matches {
                    passed = false;
                    result.failed_checks += 1;
                    if result.diagnostics.len() < 32 {
                        let observed = match observed {
                            Ok(value) => format!("{value:?}"), Err(error) => error.describe(),
                        };
                        result.diagnostics.push(format!("line {} {}(c{}) expected {expected:?}, got {observed}",
                            case.line, FORMS[form_index], column + 1));
                    }
                }
            }
        }
        result.row_statuses.push((case.line, passed));
    }
    assert_eq!(result.checks, EXPECTED_ROWS * 20);
    result
}

/// UAX15 conformance invariant2, extended to all code points and lone UTF16
/// units. NUL starter barriers permit one bulk call per form without allowing
/// neighboring marks, Hangul or surrogate units to interact across cases.
pub fn unlisted_input() -> (Vec<u16>, usize) {
    let mut part = "";
    let mut listed = BTreeSet::new();
    for line in DATA.lines() {
        let body = line.split('#').next().unwrap().trim();
        if body.starts_with('@') { part = body; continue; }
        if part == "@Part1" && !body.is_empty() {
            let first = body.split(';').next().unwrap();
            let points: Vec<_> = first.split_whitespace().collect();
            assert_eq!(points.len(), 1);
            listed.insert(u32::from_str_radix(points[0], 16).unwrap());
        }
    }
    assert!(!listed.is_empty());
    let mut input = Vec::new();
    let mut count = 0;
    for point in 0..=0x10ffff {
        if listed.contains(&point) { continue; }
        input.push(0);
        push_units(point, &mut input);
        count += 1;
    }
    (input, count)
}

pub fn run_unlisted(i: &mut Interpreter, backend: ExecutionBackend) -> (usize, Vec<String>) {
    let method = i.run_script_with_backend("String.prototype.normalize", backend).unwrap();
    let (input, count) = unlisted_input();
    let expected = JsValue::str_units(input);
    let mut failures = Vec::new();
    for form in FORMS {
        match i.binding_call(&method, expected.clone(), vec![JsValue::str(form)]) {
            Ok(value) if value == expected => {},
            Ok(_) => failures.push(format!("{form}: unlisted separated code points changed")),
            Err(error) => failures.push(format!("{form}: {}", error.describe())),
        }
    }
    (count, failures)
}
