//! Contextual colors resolved without passing through the RGBA8 paint format.
use super::*;

impl ColorValue {
    pub(crate) fn resolve(css: &str, current: Option<&Self>, dark: bool) -> Option<Self> {
        Self::resolve_depth(css,current,dark,0)
    }
    pub(super) fn resolve_depth(css:&str,current:Option<&Self>,dark:bool,depth:usize)->Option<Self> {
        if depth>=32 {return None;}
        let css=css.trim();
        if css.eq_ignore_ascii_case("currentcolor") {return current.cloned();}
        if let Some(value)=Self::parse(css) {return Some(value);}
        let lower=css.to_ascii_lowercase();let (function,inner)=split_function(&lower)?;
        if function=="light-dark" {
            let branches=words(inner,',')?;let [light,dark_branch]=branches.as_slice() else{return None};
            return Self::resolve_depth(if dark {dark_branch}else{light},current,dark,depth+1);
        }
        if function=="color-mix" {return Self::mix(inner,current,dark,depth+1);}
        if !advanced::has_from(inner) {return None;}
        let rest=inner.trim_start().get(4..)?.trim_start();
        let (source,rest)=advanced::split_source(rest)?;
        let source=Self::resolve_depth(source,current,dark,depth+1)?;
        let (channels,alpha)=advanced::split_alpha(rest)?;
        let mut parts=advanced::top_level_words(channels);
        let target=match function {
            "rgb"|"rgba"=>"srgb", "hsl"|"hsla"=>"hsl",
            "hwb"|"lab"|"lch"|"oklab"|"oklch"=>function,
            "color"=> {if parts.len()!=4 {return None;} let profile=parts.remove(0);if profile=="xyz" {"xyz-d65"}else{profile}},
            "alpha"=>source.space.as_str(),_=>return None,
        };
        let (base,missing)=source.convert_components(target)?;
        let names=match target {
            "hsl"=>["h","s","l"],"hwb"=>["h","w","b"],"lab"|"oklab"=>["l","a","b"],
            "lch"|"oklch"=>["l","c","h"],"xyz-d50"|"xyz-d65"=>["x","y","z"],_=>["r","g","b"],
        };
        let rgb_scale=if matches!(function,"rgb"|"rgba") {255.0}else{1.0};
        let keywords=[(names[0],base[0]*rgb_scale),(names[1],base[1]*rgb_scale),(names[2],base[2]*rgb_scale),("alpha",source.alpha.unwrap_or(0.0))];
        let alpha=match alpha {
            None=>source.alpha,
            Some(text) if text.eq_ignore_ascii_case("none")=>None,
            Some(text) if text.eq_ignore_ascii_case("alpha")=>source.alpha,
            Some(text)=>Some(advanced::number_channel(text,&keywords,1.0,1.0)?.clamp(0.0,1.0)),
        };
        let channels=if function=="alpha" {if !parts.is_empty() || alpha.is_none() && !rest.contains('/') {return None;}source.channels}
        else {
            if parts.len()!=3 {return None;}
            let mut channels=[None;3];
            for i in 0..3 {
                channels[i]=if parts[i].eq_ignore_ascii_case("none") || (parts[i]==names[i] && missing[i]) {None} else {
                    let value=if function=="color" {advanced::number_channel(parts[i],&keywords,1.0,1.0)?}
                        else {advanced::channel_value(function,i,parts[i],&keywords)?};
                    if !value.is_finite(){return None;}
                    Some(value/rgb_scale)
                };
            }
            channels
        };
        Some(Self{space:target.into(),channels,alpha,legacy:false})
    }
}
