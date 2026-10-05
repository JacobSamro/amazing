use amazing_html::sandbox::{SandboxingFlags as F, SandboxRestriction as R};
use amazing_html::link_rel::{parse_link_rel, hint_attribute_triggers, LinkRelation as L, LinkAttribute as A, PriorFetch as P};
fn u(value: &str) -> Vec<u16> { value.encode_utf16().collect() }
const ALL: [R;16] = [R::Navigation,R::AuxiliaryNavigation,R::TopNavigationWithoutActivation,
    R::TopNavigationWithActivation,R::Origin,R::Forms,R::PointerLock,R::Scripts,R::AutomaticFeatures,
    R::DocumentDomain,R::PropagateToAuxiliary,R::Modals,R::OrientationLock,R::Presentation,R::Downloads,R::CustomProtocolsNavigation];
#[test]
fn sandbox_absence_unknown_tokens_and_non_ascii_never_alias_permissions() {
    assert!(F::from_attribute(None).is_empty());
    let full=F::parse_directive(&[]);assert_eq!(F::from_attribute(Some(&[])),full);
    assert_eq!(F::parse_directive(&u("unknown\u{a0}allow-scripts")),full);
    assert_eq!(F::parse_directive(&[0xd800,32,0x212a]),full);
    let flags=F::parse_directive(&u("ALLOW-SCRIPTS\tallow-same-origin\nallow-scripts"));
    for r in [R::Scripts,R::AutomaticFeatures,R::Origin] {assert!(!flags.contains(r));}
    for r in [R::Navigation,R::DocumentDomain,R::Forms] {assert!(flags.contains(r));}
}
#[test]
fn all_allowance_combinations_preserve_unconditional_and_parent_restrictions() {
    let cases:&[(&str,&[R])]=&[
        ("allow-popups",&[R::AuxiliaryNavigation,R::CustomProtocolsNavigation]),
        ("allow-top-navigation",&[R::TopNavigationWithoutActivation,R::TopNavigationWithActivation,R::CustomProtocolsNavigation]),
        ("allow-top-navigation-by-user-activation",&[R::TopNavigationWithActivation]),
        ("allow-same-origin",&[R::Origin]),("allow-forms",&[R::Forms]),("allow-pointer-lock",&[R::PointerLock]),
        ("allow-scripts",&[R::Scripts,R::AutomaticFeatures]),("allow-popups-to-escape-sandbox",&[R::PropagateToAuxiliary]),
        ("allow-modals",&[R::Modals]),("allow-orientation-lock",&[R::OrientationLock]),("allow-presentation",&[R::Presentation]),
        ("allow-downloads",&[R::Downloads]),("allow-top-navigation-to-custom-protocols",&[R::CustomProtocolsNavigation])];
    for &(token,removed) in cases {let f=F::parse_directive(&u(token));for r in ALL {assert_eq!(f.contains(r),!removed.contains(&r),"{token} {r:?}");}}
    for mask in 0usize..(1usize<<cases.len()) {
        let text=cases.iter().enumerate().filter(|(i,_)|mask&(1usize<<*i)!=0).map(|(_,r)|r.0).collect::<Vec<_>>().join(" ");
        let f=F::parse_directive(&u(&text));assert!(f.contains(R::Navigation));assert!(f.contains(R::DocumentDomain));
        for (i,(token,_)) in cases.iter().enumerate() {if mask&(1usize<<i)==0 {
            let added=F::parse_directive(&u(&format!("{text} {token}")));
            for r in ALL {assert!(!added.contains(r)||f.contains(r));}
        }}
    }
    let iframe=F::parse_directive(&u("allow-scripts allow-same-origin"));let parent=F::parse_directive(&u("allow-forms"));
    let active=F::creation(F::EMPTY,Some((iframe,parent)));
    for r in [R::Scripts,R::Forms,R::Origin] {assert!(active.contains(r));}
    assert_eq!(F::creation(parent,None),parent);assert_eq!(F::creation(parent,Some((F::EMPTY,F::EMPTY))),F::EMPTY);
}
#[test]
fn all_nine_existing_link_keywords_keep_first_order_and_exact_ascii() {
    assert_eq!(parse_link_rel(&u("PRELOAD icon preLOAD unknown stylesheet modulepreload manifest prefetch preconnect dns-prefetch alternate")),
        vec![L::Preload,L::Icon,L::Stylesheet,L::ModulePreload,L::Manifest,L::Prefetch,L::Preconnect,L::DnsPrefetch,L::Alternate]);
    assert!(parse_link_rel(&u("preload\u{a0}icon")).is_empty());
    assert!(parse_link_rel(&[0xd800,112,114,101,108,111,97,100]).is_empty());
    for r in L::ADVERTISED_EXISTING {assert_eq!(parse_link_rel(&u(r.keyword())),vec![r]);}
}
#[test]
fn hint_changes_keep_module_map_and_conditional_preload_rules_distinct() {
    assert_eq!(hint_attribute_triggers(L::ModulePreload,A::Href,P::Obtained),Some(true));
    for a in [A::As,A::CrossOrigin,A::ReferrerPolicy,A::Type,A::Media] {assert_eq!(hint_attribute_triggers(L::ModulePreload,a,P::NotAttempted),Some(false));}
    for r in [L::Preconnect,L::Prefetch] {assert_eq!(hint_attribute_triggers(r,A::CrossOrigin,P::Obtained),Some(true));}
    assert_eq!(hint_attribute_triggers(L::DnsPrefetch,A::CrossOrigin,P::Obtained),Some(false));
    assert_eq!(hint_attribute_triggers(L::Preload,A::As,P::Obtained),Some(true));
    assert_eq!(hint_attribute_triggers(L::Preload,A::Type,P::UnsupportedType),Some(true));assert_eq!(hint_attribute_triggers(L::Preload,A::Type,P::Obtained),Some(false));
    assert_eq!(hint_attribute_triggers(L::Preload,A::Media,P::MediaMismatch),Some(true));assert_eq!(hint_attribute_triggers(L::Preload,A::Media,P::Obtained),Some(false));
}
