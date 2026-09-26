//! Unicode 17.0 emoji string properties used by UnicodeSets regular expressions.
//! Source: https://www.unicode.org/Public/17.0.0/emoji/

use std::collections::BTreeMap;
use std::sync::OnceLock;

type Sets = BTreeMap<&'static str, Vec<Vec<u32>>>;
static SETS: OnceLock<Sets> = OnceLock::new();

fn parse_file(text: &'static str, sets: &mut Sets) {
    for line in text.lines() {
        let mut fields = line.split(';');
        let (Some(points), Some(property)) = (fields.next(), fields.next()) else {
            continue;
        };
        let property = property.trim();
        if !matches!(
            property,
            "Basic_Emoji"
                | "Emoji_Keycap_Sequence"
                | "RGI_Emoji_Modifier_Sequence"
                | "RGI_Emoji_Flag_Sequence"
                | "RGI_Emoji_Tag_Sequence"
                | "RGI_Emoji_ZWJ_Sequence"
        ) {
            continue;
        }
        let points = points.trim();
        if let Some((start, end)) = points.split_once("..") {
            let (Ok(start), Ok(end)) = (
                u32::from_str_radix(start, 16),
                u32::from_str_radix(end.trim(), 16),
            ) else {
                continue;
            };
            for c in start..=end {
                sets.entry(property).or_default().push(vec![c]);
            }
        } else {
            let sequence: Option<Vec<u32>> = points
                .split_whitespace()
                .map(|point| u32::from_str_radix(point, 16).ok())
                .collect();
            if let Some(sequence) = sequence {
                sets.entry(property).or_default().push(sequence);
            }
        }
    }
}

fn sets() -> &'static Sets {
    SETS.get_or_init(|| {
        let mut sets = BTreeMap::new();
        parse_file(include_str!("../data/emoji-sequences-17.0.txt"), &mut sets);
        parse_file(
            include_str!("../data/emoji-zwj-sequences-17.0.txt"),
            &mut sets,
        );
        let all = sets
            .values()
            .flat_map(|entries| entries.iter().cloned())
            .collect();
        sets.insert("RGI_Emoji", all);
        for entries in sets.values_mut() {
            entries.sort();
            entries.dedup();
        }
        sets
    })
}

pub(crate) fn supported(property: &str) -> bool {
    sets().contains_key(property)
}

pub(crate) fn contains(property: &str, sequence: &[u32]) -> bool {
    sets().get(property).is_some_and(|entries| {
        entries
            .binary_search_by(|item| item.as_slice().cmp(sequence))
            .is_ok()
    })
}

pub(crate) fn strings(property: &str) -> &'static [Vec<u32>] {
    sets().get(property).map_or(&[], Vec::as_slice)
}
