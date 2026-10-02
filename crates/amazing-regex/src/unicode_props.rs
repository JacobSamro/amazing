//! Unicode 17.0 General_Category, Script, Script_Extensions and binary
//! properties used by `\p{...}`/`\P{...}` regular expression property
//! escapes (ECMA-262 UnicodeMatchProperty / UnicodeMatchPropertyValue).
//! Source: https://www.unicode.org/Public/17.0.0/ucd/
//!
//! `unicode_sets` (a sibling module) owns "properties of strings" (the RGI
//! emoji sequences usable only under the `v` flag); this module owns every
//! property that matches single code points.

use std::collections::BTreeMap;
use std::sync::OnceLock;

type Ranges = Vec<(u32, u32)>;
type RangeMap = BTreeMap<&'static str, Ranges>;
type AliasMap = BTreeMap<&'static str, &'static str>;

fn parse_range(text: &str) -> Option<(u32, u32)> {
    if let Some((a, b)) = text.split_once("..") {
        Some((
            u32::from_str_radix(a, 16).ok()?,
            u32::from_str_radix(b, 16).ok()?,
        ))
    } else {
        let v = u32::from_str_radix(text, 16).ok()?;
        Some((v, v))
    }
}

/// Parses `start[..end];Property` lines (comments/blanks already stripped
/// out of the embedded data files) into `map[Property]`.
fn parse_ranges(text: &'static str, map: &mut RangeMap) {
    for line in text.lines() {
        let mut fields = line.split(';');
        let (Some(range), Some(prop)) = (fields.next(), fields.next()) else {
            continue;
        };
        if let Some(span) = parse_range(range) {
            map.entry(prop).or_default().push(span);
        }
    }
}

fn merge(ranges: &mut Ranges) {
    ranges.sort_unstable();
    let mut merged: Ranges = Vec::with_capacity(ranges.len());
    for &(start, end) in ranges.iter() {
        match merged.last_mut() {
            Some((_, last_end)) if start <= last_end.saturating_add(1) => {
                *last_end = (*last_end).max(end);
            }
            _ => merged.push((start, end)),
        }
    }
    *ranges = merged;
}

fn in_ranges(ranges: &[(u32, u32)], cp: u32) -> bool {
    ranges
        .binary_search_by(|&(start, end)| {
            if cp < start {
                std::cmp::Ordering::Greater
            } else if cp > end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn complement(ranges: &[(u32, u32)]) -> Ranges {
    let mut out = Vec::new();
    let mut next = 0u32;
    for &(start, end) in ranges {
        if start > next {
            out.push((next, start - 1));
        }
        next = next.max(end.saturating_add(1));
        if next > 0x10_FFFF {
            return out;
        }
    }
    if next <= 0x10_FFFF {
        out.push((next, 0x10_FFFF));
    }
    out
}

/// General_Category, keyed by its two-letter abbreviation (e.g. `Lu`, `Cn`),
/// plus the eight derived one/two-letter group values (`L`, `LC`, `M`, `N`,
/// `P`, `S`, `Z`, `C`) that ECMA-262 also accepts for `General_Category`.
fn general_category() -> &'static RangeMap {
    static MAP: OnceLock<RangeMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = RangeMap::new();
        // Split across two files (each under the project's 4000-line cap);
        // concatenation reproduces the single UCD source file exactly.
        parse_ranges(
            include_str!("../data/DerivedGeneralCategory.1.txt"),
            &mut map,
        );
        parse_ranges(
            include_str!("../data/DerivedGeneralCategory.2.txt"),
            &mut map,
        );
        for ranges in map.values_mut() {
            merge(ranges);
        }
        const GROUPS: &[(&str, &[&str])] = &[
            ("L", &["Lu", "Ll", "Lt", "Lm", "Lo"]),
            ("LC", &["Lu", "Ll", "Lt"]),
            ("M", &["Mn", "Mc", "Me"]),
            ("N", &["Nd", "Nl", "No"]),
            ("P", &["Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po"]),
            ("S", &["Sm", "Sc", "Sk", "So"]),
            ("Z", &["Zs", "Zl", "Zp"]),
            ("C", &["Cc", "Cf", "Cs", "Co", "Cn"]),
        ];
        for &(name, members) in GROUPS {
            let mut ranges: Ranges = members
                .iter()
                .filter_map(|m| map.get(*m))
                .flatten()
                .copied()
                .collect();
            merge(&mut ranges);
            map.insert(name, ranges);
        }
        map
    })
}

/// Script, keyed by its long value name (e.g. `Latin`, `Common`). Unicode's
/// Scripts.txt only lists code points that have an assigned script -- it
/// never spells out `Unknown` (Zzzz) for the rest of the codespace, the way
/// DerivedGeneralCategory.txt spells out `Cn` -- but ECMA-262 still accepts
/// `\p{Script=Unknown}`/`\p{scx=Unknown}` as valid property-value syntax
/// (PropertyValueAliases.txt has the `sc;Zzzz;Unknown` alias row,
/// built-ins/RegExp/property-escapes/generated/Script(_Extensions)_-_Unknown.js).
/// So `Unknown` here is computed the same way `assigned()` computes its
/// complement of `Cn` below: whatever code point isn't in any other script's
/// range is `Unknown`, by construction covering the entire codespace exactly
/// once between this map's values.
fn scripts() -> &'static RangeMap {
    static MAP: OnceLock<RangeMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = RangeMap::new();
        parse_ranges(include_str!("../data/Scripts.txt"), &mut map);
        for ranges in map.values_mut() {
            merge(ranges);
        }
        let mut assigned_scripts: Ranges = map.values().flatten().copied().collect();
        merge(&mut assigned_scripts);
        map.insert("Unknown", complement(&assigned_scripts));
        map
    })
}

/// Script_Extensions explicit overrides, keyed by long script name, plus the
/// union of every overridden code point (any code point listed against any
/// script in ScriptExtensions.txt), used to know when to fall back to the
/// plain Script value below.
fn script_extensions() -> &'static (RangeMap, Ranges) {
    static DATA: OnceLock<(RangeMap, Ranges)> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut map = RangeMap::new();
        let mut overridden = Ranges::new();
        for line in include_str!("../data/ScriptExtensions.txt").lines() {
            let mut fields = line.split(';');
            let (Some(range), Some(codes)) = (fields.next(), fields.next()) else {
                continue;
            };
            let Some(span) = parse_range(range) else {
                continue;
            };
            overridden.push(span);
            for code in codes.split_whitespace() {
                let long = sc_value_alias().get(code).copied().unwrap_or(code);
                map.entry(long).or_default().push(span);
            }
        }
        for ranges in map.values_mut() {
            merge(ranges);
        }
        merge(&mut overridden);
        (map, overridden)
    })
}

/// Binary (single code-point) properties, combined from PropList,
/// DerivedCoreProperties, DerivedBinaryProperties, DerivedNormalizationProps
/// (just Changes_When_NFKC_Casefolded) and emoji-data, keyed by long name.
fn binary() -> &'static RangeMap {
    static MAP: OnceLock<RangeMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = RangeMap::new();
        for text in [
            include_str!("../data/PropList.txt"),
            // Split across four files (each under the project's 4000-line
            // cap); concatenation reproduces the single UCD source file
            // exactly.
            include_str!("../data/DerivedCoreProperties.1.txt"),
            include_str!("../data/DerivedCoreProperties.2.txt"),
            include_str!("../data/DerivedCoreProperties.3.txt"),
            include_str!("../data/DerivedCoreProperties.4.txt"),
            include_str!("../data/DerivedBinaryProperties.txt"),
            include_str!("../data/DerivedNormalizationProps.txt"),
            include_str!("../data/emoji-data.txt"),
        ] {
            parse_ranges(text, &mut map);
        }
        for ranges in map.values_mut() {
            merge(ranges);
        }
        map.insert("Any", vec![(0, 0x10_FFFF)]);
        map.insert("ASCII", vec![(0, 0x7F)]);
        map
    })
}

fn assigned() -> &'static [(u32, u32)] {
    static RANGES: OnceLock<Ranges> = OnceLock::new();
    RANGES
        .get_or_init(|| {
            let unassigned = general_category()
                .get("Cn")
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            complement(unassigned)
        })
        .as_slice()
}

/// Property NAME aliases (`Alpha` -> `Alphabetic`, `gc` -> `General_Category`,
/// `sc` -> `Script`, `scx` -> `Script_Extensions`, ...), from PropertyAliases.txt.
fn name_alias() -> &'static AliasMap {
    static MAP: OnceLock<AliasMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = AliasMap::new();
        for line in include_str!("../data/PropertyAliases.txt").lines() {
            let fields: Vec<&str> = line.split(';').collect();
            let Some(&canonical) = fields.get(1) else {
                continue;
            };
            for &alias in &fields {
                map.insert(alias, canonical);
            }
        }
        map
    })
}

/// General_Category VALUE aliases (`Letter` -> `L`, `Uppercase_Letter` ->
/// `Lu`, `cntrl` -> `Cc`, ...), from the `gc ;` lines of
/// PropertyValueAliases.txt. Maps every alias (including the short form
/// itself) to the short two-letter/one-letter abbreviation used as the key
/// in `general_category()`.
fn gc_value_alias() -> &'static AliasMap {
    static MAP: OnceLock<AliasMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = AliasMap::new();
        for line in include_str!("../data/PropertyValueAliases.txt").lines() {
            let fields: Vec<&str> = line.split(';').collect();
            if fields.first() != Some(&"gc") {
                continue;
            }
            let Some(&short) = fields.get(1) else {
                continue;
            };
            for &alias in &fields[1..] {
                map.insert(alias, short);
            }
        }
        map
    })
}

/// Script/Script_Extensions VALUE aliases (`Latn` -> `Latin`, `Grek` ->
/// `Greek`, ...), from the `sc ;` lines of PropertyValueAliases.txt. Maps
/// every alias to the long form used as the key in `scripts()`.
fn sc_value_alias() -> &'static AliasMap {
    static MAP: OnceLock<AliasMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = AliasMap::new();
        for line in include_str!("../data/PropertyValueAliases.txt").lines() {
            let fields: Vec<&str> = line.split(';').collect();
            if fields.first() != Some(&"sc") {
                continue;
            }
            let Some(&long) = fields.get(2).or_else(|| fields.get(1)) else {
                continue;
            };
            for &alias in &fields[1..] {
                map.insert(alias, long);
            }
        }
        map
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Lookup {
    Plain(&'static [(u32, u32)]),
    ScriptExtensions {
        explicit: &'static [(u32, u32)],
        overridden: &'static [(u32, u32)],
        base: &'static [(u32, u32)],
    },
}

pub(crate) fn lookup(expr: &str) -> Option<Lookup> {
    if let Some((name, value)) = expr.split_once('=') {
        let canonical = name_alias().get(name).copied().unwrap_or(name);
        return match canonical {
            "General_Category" => {
                let short = gc_value_alias().get(value).copied().unwrap_or(value);
                general_category()
                    .get(short)
                    .map(|r| Lookup::Plain(r.as_slice()))
            }
            "Script" => {
                let long = sc_value_alias().get(value).copied().unwrap_or(value);
                scripts().get(long).map(|r| Lookup::Plain(r.as_slice()))
            }
            "Script_Extensions" => {
                let long = sc_value_alias().get(value).copied().unwrap_or(value);
                script_extensions()
                    .0
                    .get_key_value(long)
                    .or_else(|| scripts().get_key_value(long))
                    .map(|(&key, _)| {
                        let (overrides, overridden) = script_extensions();
                        Lookup::ScriptExtensions {
                            explicit: overrides.get(key).map(Vec::as_slice).unwrap_or(&[]),
                            overridden,
                            base: scripts().get(key).map(Vec::as_slice).unwrap_or(&[]),
                        }
                    })
            }
            _ => None,
        };
    }
    match expr {
        "Any" | "ASCII" => binary().get(expr).map(|r| Lookup::Plain(r.as_slice())),
        "Assigned" => Some(Lookup::Plain(assigned())),
        _ => {
            if let Some(&short) = gc_value_alias().get(expr) {
                if let Some(ranges) = general_category().get(short) {
                    return Some(Lookup::Plain(ranges.as_slice()));
                }
            }
            let canonical = name_alias().get(expr).copied().unwrap_or(expr);
            binary().get(canonical).map(|r| Lookup::Plain(r.as_slice()))
        }
    }
}

pub(crate) fn supported(expr: &str) -> bool {
    lookup(expr).is_some()
}

pub(crate) fn contains(expr: &str, cp: u32) -> bool {
    lookup(expr).is_some_and(|property| property.contains(cp))
}

impl Lookup {
    pub(crate) fn contains(self, cp: u32) -> bool {
        match self {
            Lookup::Plain(ranges) => in_ranges(ranges, cp),
            Lookup::ScriptExtensions {
                explicit,
                overridden,
                base,
            } => in_ranges(explicit, cp) || (!in_ranges(overridden, cp) && in_ranges(base, cp)),
        }
    }
}
