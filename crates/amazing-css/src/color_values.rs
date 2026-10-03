//! CSS Color 4 absolute value grammar and precision-preserving serialization.
//! This is separate from the existing RGBA8 paint representation.
use super::*;
use crate::syntax::{Token, tokenize};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColorValue {
    pub(crate) space: String,
    pub(crate) channels: [Option<f64>; 3],
    pub(crate) alpha: Option<f64>,
    legacy: bool,
}
#[derive(Clone, Copy)]
enum Component { Number(f64), Percent(f64), Missing }
fn component(css: &str, missing: bool) -> Option<Component> {
    if missing && css.trim().eq_ignore_ascii_case("none") { return Some(Component::Missing); }
    let tokens = tokenize(css);
    let significant: Vec<_> = tokens.iter().filter(|t| !matches!(t, Token::Whitespace)).collect();
    match significant.as_slice() {
        [Token::Number(n)] if n.is_finite() => Some(Component::Number(*n)),
        [Token::Percentage(n)] if n.is_finite() => Some(Component::Percent(*n)),
        _ => crate::math::computed_number(css).filter(|n| n.is_finite()).map(Component::Number)
            .or_else(|| crate::math::computed_percentage(css).filter(|n| n.is_finite()).map(Component::Percent)),
    }
}
fn number(component: Component, percent_scale: f64) -> Option<f64> {
    match component {
        Component::Number(n) => Some(n),
        Component::Percent(n) => Some(n * percent_scale / 100.0),
        Component::Missing => None,
    }
}
fn angle(css: &str, missing: bool) -> Option<Option<f64>> {
    if missing && css.eq_ignore_ascii_case("none") { return Some(None); }
    let tokens: Vec<_> = tokenize(css).into_iter().filter(|t| !matches!(t, Token::Whitespace)).collect();
    let value = match tokens.as_slice() {
        [Token::Number(n)] => *n,
        [Token::Dimension(n, unit)] => n * match unit.to_ascii_lowercase().as_str() {
            "deg" => 1.0, "grad" => 0.9, "rad" => 180.0/std::f64::consts::PI,
            "turn" => 360.0, _ => return None,
        },
        _ => crate::math::computed_angle(css)?.to_degrees(),
    };
    value.is_finite().then_some(Some(value.rem_euclid(360.0)))
}
fn words(css: &str, delimiter: char) -> Option<Vec<String>> {
    let mut result=Vec::new();let mut depth=0usize;let mut start=0usize;
    for (i,ch) in css.char_indices() {
        match ch {'('=>depth+=1,')'=>depth=depth.checked_sub(1)?,_=>{}}
        if depth==0 && if delimiter==' ' {ch.is_ascii_whitespace()} else {ch==delimiter} {
            let part=css[start..i].trim();
            if !part.is_empty() {result.push(part.to_owned());}
            else if delimiter!=' ' {return None;}
            start=i+ch.len_utf8();
        }
    }
    if depth!=0 {return None;}
    let tail=css[start..].trim();
    if !tail.is_empty() {result.push(tail.to_owned());}
    else if delimiter!=' ' {return None;}
    Some(result)
}
fn components(inner: &str, commas: bool) -> Option<(Vec<String>, Option<f64>, bool)> {
    let comma_parts=words(inner,',')?;
    let legacy=comma_parts.len()>1;
    let (channels, alpha)=if legacy {
        if !commas || inner.contains('/') || !matches!(comma_parts.len(),3|4) {return None;}
        let alpha=if comma_parts.len()==4 {number(component(&comma_parts[3],false)?,1.0)} else {Some(1.0)};
        (comma_parts[..3].to_vec(),alpha)
    } else {
        let parts=words(inner,'/')?;
        if !matches!(parts.len(),1|2) {return None;}
        let alpha=if parts.len()==2 {number(component(&parts[1],true)?,1.0)} else {Some(1.0)};
        (words(&parts[0],' ')?,alpha)
    };
    Some((channels,alpha.map(|n|n.clamp(0.0,1.0)),legacy))
}
impl ColorValue {
    pub(crate) fn parse(css: &str) -> Option<Self> { Self::parse_mode(css,false) }
    fn parse_mode(css:&str,preserve_hue:bool)->Option<Self> {
        let css=css.trim();let lower=css.to_ascii_lowercase();
        // System roles require the actual computed system palette, not this parser.
        if parse_system_color(css).is_some() {return None;}
        if let Some([r,g,b,a])=lookup_named(&lower).or_else(|| css.strip_prefix('#').and_then(parse_hex)) {
            let alpha=if a==255 {1.0} else if a==0 {0.0} else {
                let hundredth=(f64::from(a)*100.0/255.0).round()/100.0;
                if (hundredth*255.0).round()==f64::from(a) {hundredth} else {(f64::from(a)*1000.0/255.0).round()/1000.0}
            };
            return Some(Self{space:"srgb".into(),channels:[r,g,b].map(|n|Some(f64::from(n)/255.0)),alpha:Some(alpha),legacy:true});
        }
        if !matches!(tokenize(css).iter().find(|t|!matches!(t,Token::Whitespace)),Some(Token::Function(_))) {return None;}
        let (name,inner)=split_function(&lower)?;
        if inner.trim_start().starts_with("from ") {return None;}
        let (mut parts,alpha,legacy)=components(inner,matches!(name,"rgb"|"rgba"|"hsl"|"hsla"))?;
        let mut space=name;
        if name=="color" {
            if legacy || parts.len()!=4 {return None;}
            let profile=parts.remove(0);
            space=match profile.as_str() {"srgb"=>"srgb","srgb-linear"=>"srgb-linear","display-p3"=>"display-p3","display-p3-linear"=>"display-p3-linear",
                "a98-rgb"=>"a98-rgb","prophoto-rgb"=>"prophoto-rgb","rec2020"=>"rec2020",
                "xyz"|"xyz-d65"=>"xyz-d65","xyz-d50"=>"xyz-d50",_=>return None};
        }
        if parts.len()!=3 {return None;}
        let mut channels=[None;3];
        match name {
            "rgb"|"rgba" => {
                let components=parts.iter().map(|p|component(p,!legacy)).collect::<Option<Vec<_>>>()?;
                if legacy && components.iter().any(|c|matches!(c,Component::Percent(_)))
                    && components.iter().any(|c|matches!(c,Component::Number(_))) {return None;}
                for (i,c) in components.into_iter().enumerate() {channels[i]=number(c,255.0).map(|n|n.clamp(0.0,255.0)/255.0);}
                space="srgb";
            }
            "hsl"|"hsla"|"hwb" => {
                let h=angle(&parts[0],!legacy)?;
                let a=component(&parts[1],!legacy)?;let b=component(&parts[2],!legacy)?;
                if legacy && (!matches!(a,Component::Percent(_)) || !matches!(b,Component::Percent(_))) {return None;}
                let a=number(a,100.0).map(|n|n.clamp(0.0,100.0));
                let b=number(b,100.0).map(|n|n.clamp(0.0,100.0));
                if preserve_hue || h.is_none() || a.is_none() || b.is_none() || alpha.is_none() {
                    channels=[h,a,b];space=if name=="hwb" {"hwb"} else {"hsl"};
                } else {
                    let rgb=if name=="hwb" {advanced::hwb_rgb(h.unwrap(),a.unwrap()/100.0,b.unwrap()/100.0)}
                        else {advanced::hsl_rgb(h.unwrap(),a.unwrap(),b.unwrap())};
                    channels=rgb.map(Some);space="srgb";
                }
            }
            "lab"|"lch"|"oklab"|"oklch" => {
                if legacy {return None;}
                let ok=name.starts_with("ok");let polar=name.ends_with("lch");
                channels[0]=number(component(&parts[0],true)?,if ok {1.0}else{100.0}).map(|n|n.clamp(0.0,if ok {1.0}else{100.0}));
                channels[1]=number(component(&parts[1],true)?,if polar {if ok {0.4}else{150.0}}else if ok {0.4}else{125.0})
                    .map(|n|if polar {n.max(0.0)} else {n});
                channels[2]=if polar {angle(&parts[2],true)?}
                    else {number(component(&parts[2],true)?,if ok {0.4}else{125.0})};
            }
            "color" => {for i in 0..3 {channels[i]=number(component(&parts[i],true)?,1.0);}}
            _=>return None,
        }
        Some(Self{space:space.to_owned(),channels,alpha,legacy:name!="color" && space=="srgb"})
    }
    pub(crate) fn serialize(&self) -> String {
        let text=|n:Option<f64>|n.map(|n|if n==0.0 {"0".into()} else {n.to_string()}).unwrap_or_else(||"none".into());
        if self.legacy && self.channels.iter().all(Option::is_some) && self.alpha.is_some() {
            let values=self.channels.map(|n|text(n.map(|n|n*255.0)));
            if self.alpha==Some(1.0) {return format!("rgb({}, {}, {})",values[0],values[1],values[2]);}
            return format!("rgba({}, {}, {}, {})",values[0],values[1],values[2],text(self.alpha));
        }
        let channels=if matches!(self.space.as_str(),"hsl"|"hwb") {
            self.channels.iter().enumerate().map(|(i,n)| {
                let value=text(*n);if i>0 && n.is_some() {format!("{value}%")} else {value}
            }).collect::<Vec<_>>().join(" ")
        } else {self.channels.map(text).join(" ")};
        let alpha=if self.alpha==Some(1.0) {String::new()} else {format!(" / {}",text(self.alpha))};
        if matches!(self.space.as_str(),"lab"|"lch"|"oklab"|"oklch"|"hsl"|"hwb") {format!("{}({channels}{alpha})",self.space)}
        else {format!("color({} {channels}{alpha})",self.space)}
    }
}


#[path="color_context.rs"]
mod context;

#[cfg(test)]
#[path="color_context_tests.rs"]
mod tests;

#[path="color_mix_precise.rs"]
mod mix;
