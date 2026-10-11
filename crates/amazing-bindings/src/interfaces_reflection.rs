//! Reflecting IDL attributes that HTML defines on elements but the base
//! interface tables lacked (html/dom reflection corpus). Each table is appended
//! to the element's existing members with `with_legacy_members`.
use super::*;
use crate::host::EnumSpec;

const fn spec(keywords: &'static [&'static str], missing: &'static str, invalid: &'static str) -> EnumSpec {
    EnumSpec { keywords, non_canon: &[], missing, invalid, nullable: false }
}
static REFERRER_POLICY: EnumSpec = spec(&["", "no-referrer", "no-referrer-when-downgrade", "same-origin", "origin",
    "strict-origin", "origin-when-cross-origin", "strict-origin-when-cross-origin", "unsafe-url"], "", "");
static CROSS_ORIGIN: EnumSpec = EnumSpec { keywords: &["anonymous", "use-credentials"], non_canon: &[("", "anonymous")],
    missing: "", invalid: "anonymous", nullable: true };
static PRELOAD: EnumSpec = EnumSpec { keywords: &["none", "metadata", "auto"], non_canon: &[("", "auto")],
    missing: "metadata", invalid: "metadata", nullable: false };
static LOADING: EnumSpec = spec(&["lazy", "eager"], "eager", "eager");
static TRACK_KIND: EnumSpec = spec(&["subtitles", "captions", "descriptions", "chapters", "metadata"], "subtitles", "metadata");
static SCOPE: EnumSpec = spec(&["row", "col", "rowgroup", "colgroup"], "", "");
static LINK_AS: EnumSpec = spec(&["fetch", "audio", "document", "embed", "font", "image", "manifest", "object", "report",
    "script", "sharedworker", "style", "track", "video", "worker", "xslt"], "", "");
static INPUT_MODE: EnumSpec = spec(&["none", "text", "tel", "url", "email", "numeric", "decimal", "search"], "", "");
static ENTER_KEY_HINT: EnumSpec = spec(&["enter", "done", "go", "next", "previous", "search", "send"], "", "");
static MARQUEE_BEHAVIOR: EnumSpec = spec(&["scroll", "slide", "alternate"], "scroll", "scroll");
static MARQUEE_DIRECTION: EnumSpec = spec(&["up", "right", "down", "left"], "left", "left");

macro_rules! reflected {
    ($iface:expr; $( ($idl:literal, $attr:literal, $kind:expr) ),* $(,)?) => {
        &[ $( getter($idl, O::LegacyReflectGet($iface, $attr, $kind)),
              setter($idl, O::LegacyReflectSet($iface, $attr, $kind)), )* ]
    };
}

pub(crate) const HTML_ELEMENT_NEW: &[Member] = reflected!(I::HTMLElement;
    ("autofocus", "autofocus", LR::Boolean),
    ("inputMode", "inputmode", LR::Enum(&INPUT_MODE)),
    ("enterKeyHint", "enterkeyhint", LR::Enum(&ENTER_KEY_HINT)));

const ANCHOR_NEW: &[Member] = reflected!(I::HTMLAnchorElement;
    ("ping", "ping", LR::String), ("hreflang", "hreflang", LR::String), ("type", "type", LR::String),
    ("referrerPolicy", "referrerpolicy", LR::Enum(&REFERRER_POLICY)));
pub(crate) const ANCHOR: [Member; LEGACY_ANCHOR.len() + ANCHOR_NEW.len()] = with_legacy_members(&LEGACY_ANCHOR, ANCHOR_NEW);

const AREA_NEW: &[Member] = reflected!(I::HTMLAreaElement;
    ("alt", "alt", LR::String), ("coords", "coords", LR::String), ("shape", "shape", LR::String),
    ("ping", "ping", LR::String), ("hreflang", "hreflang", LR::String), ("type", "type", LR::String),
    ("referrerPolicy", "referrerpolicy", LR::Enum(&REFERRER_POLICY)));
pub(crate) const AREA: [Member; LEGACY_AREA.len() + AREA_NEW.len()] = with_legacy_members(&LEGACY_AREA, AREA_NEW);

const LINK_NEW: &[Member] = reflected!(I::HTMLLinkElement;
    ("crossOrigin", "crossorigin", LR::Enum(&CROSS_ORIGIN)), ("as", "as", LR::Enum(&LINK_AS)),
    ("integrity", "integrity", LR::String), ("hreflang", "hreflang", LR::String),
    ("referrerPolicy", "referrerpolicy", LR::Enum(&REFERRER_POLICY)));
pub(crate) const LINK: [Member; LEGACY_LINK.len() + LINK_NEW.len()] = with_legacy_members(&LEGACY_LINK, LINK_NEW);

const SCRIPT_NEW: &[Member] = reflected!(I::HTMLScriptElement;
    ("crossOrigin", "crossorigin", LR::Enum(&CROSS_ORIGIN)), ("integrity", "integrity", LR::String),
    ("noModule", "nomodule", LR::Boolean));
pub(crate) const SCRIPT: [Member; LEGACY_SCRIPT.len() + SCRIPT_NEW.len()] = with_legacy_members(&LEGACY_SCRIPT, SCRIPT_NEW);

const STYLE_NEW: &[Member] = reflected!(I::HTMLStyleElement; ("media", "media", LR::String));
pub(crate) const STYLE: [Member; LEGACY_STYLE.len() + STYLE_NEW.len()] = with_legacy_members(&LEGACY_STYLE, STYLE_NEW);

const META_NEW: &[Member] = reflected!(I::HTMLMetaElement; ("media", "media", LR::String));
pub(crate) const META_ELEMENT: [Member; LEGACY_META.len() + META_NEW.len()] = with_legacy_members(&LEGACY_META, META_NEW);

const IMAGE_NEW: &[Member] = reflected!(I::HTMLImageElement;
    ("crossOrigin", "crossorigin", LR::Enum(&CROSS_ORIGIN)), ("useMap", "usemap", LR::String),
    ("isMap", "ismap", LR::Boolean), ("referrerPolicy", "referrerpolicy", LR::Enum(&REFERRER_POLICY)));
pub(crate) const IMAGE_ELEMENT: [Member; LEGACY_IMAGE.len() + IMAGE_NEW.len()] = with_legacy_members(&LEGACY_IMAGE, IMAGE_NEW);

const IFRAME_NEW: &[Member] = reflected!(I::HTMLIFrameElement;
    ("name", "name", LR::String), ("allowFullscreen", "allowfullscreen", LR::Boolean),
    ("width", "width", LR::String), ("height", "height", LR::String),
    ("referrerPolicy", "referrerpolicy", LR::Enum(&REFERRER_POLICY)));
pub(crate) const IFRAME_ELEMENT: [Member; LEGACY_IFRAME.len() + IFRAME_NEW.len()] = with_legacy_members(&LEGACY_IFRAME, IFRAME_NEW);

const EMBED_NEW: &[Member] = reflected!(I::HTMLEmbedElement;
    ("src", "src", LR::Url), ("type", "type", LR::String), ("width", "width", LR::String), ("height", "height", LR::String));
pub(crate) const EMBED: [Member; LEGACY_EMBED.len() + EMBED_NEW.len()] = with_legacy_members(&LEGACY_EMBED, EMBED_NEW);

const OBJECT_NEW: &[Member] = reflected!(I::HTMLObjectElement;
    ("data", "data", LR::Url), ("type", "type", LR::String), ("name", "name", LR::String),
    ("width", "width", LR::String), ("height", "height", LR::String));
pub(crate) const OBJECT: [Member; LEGACY_OBJECT.len() + OBJECT_NEW.len()] = with_legacy_members(&LEGACY_OBJECT, OBJECT_NEW);

pub(crate) const MEDIA: &[Member] = reflected!(I::HTMLMediaElement;
    ("src", "src", LR::Url), ("crossOrigin", "crossorigin", LR::Enum(&CROSS_ORIGIN)),
    ("preload", "preload", LR::Enum(&PRELOAD)), ("autoplay", "autoplay", LR::Boolean),
    ("loop", "loop", LR::Boolean), ("controls", "controls", LR::Boolean),
    ("defaultMuted", "muted", LR::Boolean), ("loading", "loading", LR::Enum(&LOADING)));
pub(crate) const VIDEO: &[Member] = reflected!(I::HTMLVideoElement;
    ("width", "width", LR::Unsigned), ("height", "height", LR::Unsigned),
    ("poster", "poster", LR::Url), ("playsInline", "playsinline", LR::Boolean));

const SOURCE_NEW: &[Member] = reflected!(I::HTMLSourceElement; ("src", "src", LR::Url));
pub(crate) const SOURCE_ELEMENT: [Member; SOURCE.len() + SOURCE_NEW.len()] = with_legacy_members(SOURCE, SOURCE_NEW);

pub(crate) const TRACK: &[Member] = reflected!(I::HTMLTrackElement;
    ("kind", "kind", LR::Enum(&TRACK_KIND)), ("src", "src", LR::Url), ("srclang", "srclang", LR::String),
    ("label", "label", LR::String), ("default", "default", LR::Boolean));
pub(crate) const MAP: &[Member] = reflected!(I::HTMLMapElement; ("name", "name", LR::String));
pub(crate) const QUOTE: &[Member] = reflected!(I::HTMLQuoteElement; ("cite", "cite", LR::Url));
pub(crate) const MOD: &[Member] = reflected!(I::HTMLModElement;
    ("cite", "cite", LR::Url), ("dateTime", "datetime", LR::String));
pub(crate) const TIME: &[Member] = reflected!(I::HTMLTimeElement; ("dateTime", "datetime", LR::String));
pub(crate) const DATA: &[Member] = reflected!(I::HTMLDataElement; ("value", "value", LR::String));
pub(crate) const DETAILS: &[Member] = reflected!(I::HTMLDetailsElement; ("open", "open", LR::Boolean));
pub(crate) const DIALOG: &[Member] = reflected!(I::HTMLDialogElement; ("open", "open", LR::Boolean));

const COL_NEW: &[Member] = reflected!(I::HTMLTableColElement; ("span", "span", LR::Span));
pub(crate) const TABLECOL: [Member; LEGACY_TABLECOL.len() + COL_NEW.len()] = with_legacy_members(&LEGACY_TABLECOL, COL_NEW);
const CELL_NEW: &[Member] = reflected!(I::HTMLTableCellElement;
    ("abbr", "abbr", LR::String), ("headers", "headers", LR::String), ("scope", "scope", LR::Enum(&SCOPE)));
pub(crate) const TABLECELL: [Member; LEGACY_TABLECELL.len() + CELL_NEW.len()] = with_legacy_members(&LEGACY_TABLECELL, CELL_NEW);
const OLIST_NEW: &[Member] = reflected!(I::HTMLOListElement; ("type", "type", LR::String));
pub(crate) const OLIST: [Member; LEGACY_OLIST.len() + OLIST_NEW.len()] = with_legacy_members(&LEGACY_OLIST, OLIST_NEW);

pub(crate) const MARQUEE: &[Member] = reflected!(I::HTMLMarqueeElement;
    ("behavior", "behavior", LR::Enum(&MARQUEE_BEHAVIOR)), ("bgColor", "bgcolor", LR::String),
    ("direction", "direction", LR::Enum(&MARQUEE_DIRECTION)), ("height", "height", LR::String),
    ("hspace", "hspace", LR::Unsigned), ("scrollAmount", "scrollamount", LR::UnsignedDefault(6)),
    ("scrollDelay", "scrolldelay", LR::UnsignedDefault(85)), ("trueSpeed", "truespeed", LR::Boolean),
    ("vspace", "vspace", LR::Unsigned), ("width", "width", LR::String));
