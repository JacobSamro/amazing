//! CSS Images gradient grammar and computed serialization without image geometry.
use crate::computed::{function_inner, top_level_components, trim_tokens};
use crate::syntax::{Token, serialize, tokenize};

fn parts(tokens: &[Token]) -> Option<Vec<String>> {
    Some(top_level_components(tokens)?.into_iter().map(serialize).collect())
}
fn word(value: &str, expected: &str) -> bool { value.eq_ignore_ascii_case(expected) }
fn angle(value: &str) -> bool {
    matches!(trim_tokens(&tokenize(value)), [Token::Number(n)] if *n == 0.0)
        || crate::math::angle(value)
        || matches!(trim_tokens(&tokenize(value)), [Token::Dimension(n,u)] if n.is_finite()
            && matches!(u.to_ascii_lowercase().as_str(), "deg"|"grad"|"rad"|"turn"))
}
fn angle_css(value: &str) -> String {
    let tokens = tokenize(value);
    let radians = match trim_tokens(&tokens) {
        [Token::Number(n)] if *n == 0.0 => Some(0.0),
        [Token::Dimension(n,u)] => match u.to_ascii_lowercase().as_str() {
            "deg" => Some(n.to_radians()), "grad" => Some(n * std::f64::consts::PI/200.0),
            "rad" => Some(*n), "turn" => Some(n * std::f64::consts::TAU), _ => None,
        },
        _ => crate::math::computed_angle(value),
    };
    radians.filter(|n|n.is_finite()).map(|n|format!("{}deg", (n.to_degrees()*1e10).round()/1e10))
        .unwrap_or_else(||value.to_owned())
}
fn position_value(value: &str, conic: bool) -> bool {
    if conic { angle(value) || matches!(trim_tokens(&tokenize(value)), [Token::Percentage(n)] if n.is_finite())
        || crate::math::percentage(value) || angle_percentage_math(value) }
    else { crate::values::LengthPercentage::parse(value).is_some() || crate::math::length_percentage(value, true)
        || (matches!(trim_tokens(&tokenize(value)),[Token::Dimension(n,_)] if n.is_finite())
            && crate::math::length_percentage(&format!("calc({value})"),true)) }
}
fn angle_percentage_math(value: &str) -> bool {
    // Angle-percentage arithmetic has the same type algebra as
    // length-percentage. Substitute only the dimension *type* for validation;
    // never evaluate these proxy quantities or invent an image percentage basis.
    let mut tokens=tokenize(value);
    for token in &mut tokens {
        if let Token::Dimension(n,unit)=token {
            if !n.is_finite() || !matches!(unit.to_ascii_lowercase().as_str(),"deg"|"grad"|"rad"|"turn") {return false;}
            *unit="px".into();
        }
    }
    crate::math::length_percentage(&serialize(&tokens),true)
}
fn color_css(value: &str) -> Option<String> {
    if !crate::image_values::color(&tokenize(value)) { return None; }
    // Preserve functional/extended color representation and its precision.
    // Basic named/hex colors use the engine's existing typed RGBA color model.
    if matches!(trim_tokens(&tokenize(value)), [Token::Ident(_)] | [Token::Hash(_)]) {
        if let Some([r,g,b,a]) = crate::color::parse_css_color(value) {
            return Some(if a==255 {format!("rgb({r}, {g}, {b})")}
                else {format!("rgba({r}, {g}, {b}, {})", f64::from(a)/255.0)});
        }
    }
    Some(value.to_owned())
}

fn interpolation(parts: &mut Vec<String>) -> Option<Option<String>> {
    let Some(start) = parts.iter().position(|s|word(s,"in")) else { return Some(None); };
    let space=parts.get(start+1)?.to_ascii_lowercase();
    if !matches!(space.as_str(), "srgb"|"srgb-linear"|"display-p3"|"a98-rgb"|"prophoto-rgb"
        |"rec2020"|"lab"|"oklab"|"xyz"|"xyz-d50"|"xyz-d65"|"hsl"|"hwb"|"lch"|"oklch")
        && !space.starts_with("--") {return None;}
    let mut count=2;
    let mut result=format!("in {space}");
    if parts.get(start+2).is_some_and(|s|matches!(s.to_ascii_lowercase().as_str(),"shorter"|"longer"|"increasing"|"decreasing")) {
        if !matches!(space.as_str(),"hsl"|"hwb"|"lch"|"oklch") || !parts.get(start+3).is_some_and(|s|word(s,"hue")) {return None;}
        result.push_str(&format!(" {} hue",parts[start+2].to_ascii_lowercase()));count=4;
    }
    parts.drain(start..start+count);
    Some(Some(result))
}
fn geometry(tokens: &[Token], name: &str) -> Option<String> {
    let mut p=parts(tokens)?;
    let interpolation=interpolation(&mut p)?;
    let at=p.iter().position(|s|word(s,"at"));
    let position=if let Some(at)=at {
        if name.contains("linear") {return None;}
        let result=crate::computed::position_values::canonical("object-position",&p[at+1..].join(" "))?;
        p.truncate(at);
        (result!="50% 50%").then(||format!("at {result}"))
    } else {None};
    let mut out=Vec::new();
    if name.contains("linear") {
        match p.as_slice() {
            [a] if angle(a) => {let a=angle_css(a);if a!="180deg" {out.push(a);}},
            [to, side] if word(to,"to") && matches!(side.to_ascii_lowercase().as_str(),"left"|"right"|"top"|"bottom") => {
                if !word(side,"bottom") {out.push(format!("to {}",side.to_ascii_lowercase()));}
            },
            [to,a,b] if word(to,"to") => {
                let horizontal=|v:&str|word(v,"left")||word(v,"right");
                let vertical=|v:&str|word(v,"top")||word(v,"bottom");
                if horizontal(a)&&vertical(b) {out.push(format!("to {} {}",a.to_ascii_lowercase(),b.to_ascii_lowercase()));}
                else if horizontal(b)&&vertical(a) {out.push(format!("to {} {}",b.to_ascii_lowercase(),a.to_ascii_lowercase()));}
                else {return None;}
            },
            [] if interpolation.is_some()=>{}, _=>return None,
        }
    } else if name.contains("conic") {
        match p.as_slice() {
            []=>{}, [from,a] if word(from,"from")&&angle(a)=>{
                let a=angle_css(a);if a!="0deg" {out.push(format!("from {a}"));}
            }, _=>return None,
        }
    } else {
        if !crate::image_values::radial(tokens) {return None;}
        let mut shape=None;let mut extent=None;let mut radii=Vec::new();
        for value in p {
            match value.to_ascii_lowercase().as_str() {
                "circle"|"ellipse"=>{if shape.replace(value.to_ascii_lowercase()).is_some(){return None;}},
                "closest-side"|"closest-corner"|"farthest-side"|"farthest-corner"=>{
                    if extent.replace(value.to_ascii_lowercase()).is_some(){return None;}
                }, _=>radii.push(value),
            }
        }
        if let Some(shape)=shape {
            if (shape=="circle"&&radii.is_empty()) || (shape=="ellipse"&&radii.len()==1) {out.push(shape);}
        }
        if let Some(extent)=extent {if extent!="farthest-corner" {out.push(extent);}}
        out.extend(radii);
    }
    out.extend(position);out.extend(interpolation);
    Some(out.join(" "))
}

/// Validate the complete stop list and serialize grammar defaults and positions.
pub(crate) fn canonical(tokens: &[Token], name: &str) -> Option<String> {
    let inner=function_inner(tokens,name)?;
    let groups=crate::image_values::groups(inner)?;
    let conic=name.contains("conic");
    let mut out=Vec::new();let mut stops=0;let mut hint=false;
    for (index,group) in groups.iter().enumerate() {
        let p=parts(group)?;
        let colors:Vec<_>=p.iter().enumerate().filter_map(|(i,s)|color_css(s).map(|c|(i,c))).collect();
        if colors.len()==1 {
            let (color_index,color)=&colors[0];
            let positions:Vec<_>=p.iter().enumerate().filter(|(i,_)|i!=color_index).map(|(_,s)|s).collect();
            if positions.len()>2 || !positions.iter().all(|s|position_value(s,conic)) {return None;}
            let mut stop=color.clone();
            for position in positions {stop.push(' ');stop.push_str(&if conic&&angle(position){angle_css(position)}else{position.to_owned()});}
            out.push(stop);stops+=1;hint=false;
        } else if colors.is_empty() && index==0 {
            let prelude=geometry(group,name)?;if !prelude.is_empty(){out.push(prelude);}
        } else if colors.is_empty() && p.len()==1 && stops>0 && index+1<groups.len()
            && !hint && position_value(&p[0],conic) {
            out.push(if conic&&angle(&p[0]){angle_css(&p[0])}else{p[0].clone()});hint=true;
        } else {return None;}
    }
    (stops>0&&!hint).then(||format!("{}({})",name.to_ascii_lowercase(),out.join(", ")))
}

pub(crate) fn computed(css: &str, font: f64, root: f64) -> String {
    fn clamp_radial_radii(tokens:&mut [Token]) {
        // Literal negative radii were rejected at declaration validation.
        // Negative results of valid math are clamped at computed time; stop
        // positions and `at` coordinates have no such nonnegative range.
        let mut depth=0;let mut end=tokens.len().saturating_sub(1);
        for (index,token) in tokens.iter().enumerate().skip(2) {
            match token {Token::OpenParen=>depth+=1,Token::CloseParen if depth>0=>depth-=1,
                Token::Comma if depth==0=>{end=index;break;},_=>{}}
        }
        if parts(&tokens[2..end]).is_some_and(|p|p.iter().any(|s|color_css(s).is_some())) {return;}
        for token in &mut tokens[2..end] {
            if matches!(token,Token::Ident(s) if word(s,"at")||word(s,"in")) {break;}
            if let Token::Dimension(n,unit)=token {if unit=="px"&&*n<0.0 {*n=0.0;}}
        }
    }
    fn walk(tokens: &[Token], font: f64, root: f64, nesting:usize) -> Vec<Token> {
        if nesting>=64 {return tokens.to_vec();}
        let mut out=Vec::new();let mut index=0;
        while index<tokens.len() {
            if let Token::Function(name)=&tokens[index] {
                if tokens.get(index+1)==Some(&Token::OpenParen) {
                    let mut depth=0;let mut end=None;
                    for (j,token) in tokens.iter().enumerate().skip(index+1) {
                        if *token==Token::OpenParen {depth+=1;}
                        if *token==Token::CloseParen {depth-=1;if depth==0 {end=Some(j);break;}}
                    }
                    if let Some(end)=end {
                        let inner=walk(&tokens[index+2..end],font,root,nesting+1);
                        let mut full=vec![Token::Function(name.clone()),Token::OpenParen];
                        full.extend(inner);full.push(Token::CloseParen);
                        if matches!(name.to_ascii_lowercase().as_str(),"calc"|"min"|"max"|"clamp")
                            && full.iter().all(|token|!matches!(token,Token::Dimension(_,u) if u!="px")) {
                            let css=serialize(&full);
                            if crate::math::length_percentage(&css,false) {
                                if let Some(px)=crate::math::font_size_px(&css,font,root).filter(|n|n.is_finite()) {
                                    full=vec![Token::Dimension(px,"px".into())];
                                }
                            }
                        }
                        if name.to_ascii_lowercase().ends_with("gradient") {
                            if name.to_ascii_lowercase().contains("radial") {clamp_radial_radii(&mut full);}
                            if let Some(css)=canonical(&full,name) {full=tokenize(&css);}
                        }
                        out.extend(full);index=end+1;continue;
                    }
                }
            }
            let mut token=tokens[index].clone();
            if let Token::Dimension(_,_) =&token {
                let px=match crate::values::LengthPercentage::parse(&serialize(&[token.clone()])) {
                    Some(crate::values::LengthPercentage::Px(n))=>Some(n),
                    Some(crate::values::LengthPercentage::Relative(n,crate::values::RelativeUnit::Em))=>Some(n*font),
                    Some(crate::values::LengthPercentage::Relative(n,crate::values::RelativeUnit::Rem))=>Some(n*root),
                    _=>None,
                };
                if let Some(px)=px.filter(|n|n.is_finite()) {token=Token::Dimension(px,"px".into());}
            }
            out.push(token);index+=1;
        }
        out
    }
    serialize(&walk(&tokenize(css),font,root,0))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid(css:&str)->Option<String> {
        let tokens=tokenize(css);let Token::Function(name)=tokens.first()? else{return None;};canonical(&tokens,name)
    }
    #[test]
    fn gradient_positions_defaults_and_repeating_families_share_grammar() {
        for name in ["radial-gradient","repeating-radial-gradient","conic-gradient","repeating-conic-gradient"] {
            for (position,expected) in [("center",""),("bottom 10% right 20%","at 80% 90%, "),("10%","at 10% 50%, ")] {
                let css=format!("{name}(at {position}, red, blue)");
                assert_eq!(valid(&css),Some(format!("{name}({expected}rgb(255, 0, 0), rgb(0, 0, 255))")));
            }
        }
        assert_eq!(valid("linear-gradient(to bottom, red, blue)"),Some("linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255))".into()));
        assert_eq!(valid("conic-gradient(from 0.25turn at left, red 0deg, blue 100%)"),Some("conic-gradient(from 90deg at 0% 50%, rgb(255, 0, 0) 0deg, rgb(0, 0, 255) 100%)".into()));
    }
    #[test]
    fn complete_gradient_lists_reject_invalid_geometry_stops_and_hints() {
        for css in ["linear-gradient()","linear-gradient(to left right, red, blue)","linear-gradient(red 1px 2px 3px, blue)",
            "linear-gradient(20%, red, blue)","linear-gradient(red, 20%, 30%, blue)","linear-gradient(red, 20%)",
            "radial-gradient(at top left bottom, red, blue)","conic-gradient(from 2px, red, blue)",
            "conic-gradient(red 2px, blue)","radial-gradient(circle -2px, red, blue)",
            "linear-gradient(in srgb longer hue, red, blue)"] {assert!(valid(css).is_none(),"{css}");}
        for css in ["linear-gradient(red)","linear-gradient(red -10px, 30%, blue 100px 200px)",
            "radial-gradient(ellipse 20% 30% at right top, red, blue)","linear-gradient(in oklch longer hue, red, blue)",
            "conic-gradient(red calc(90deg + 50%), blue)","conic-gradient(red, calc(100% - 45deg), blue)"] {
            assert!(valid(css).is_some(),"{css}");
        }
        let mut dom=amazing_dom::Dom::new();
        let node=dom.create_element(amazing_dom::NodeIdx(0),"li");
        let styled=crate::compute(&dom,&[crate::parse_stylesheet(
            "li {list-style-position:outside;list-style-image:url(good.png);background-image:url(good.png);\
            list-style:inside linear-gradient(to left right, red, blue);\
            background-image:conic-gradient(red 2px, blue);\
            list-style-image:linear-gradient(red 1px 2px 3px, blue)}")]);
        let style=styled.get(node).unwrap();
        assert_eq!(style.get("list-style-position"),Some("outside"));
        assert!(style.get("list-style-image").unwrap().contains("good.png"));
        assert!(style.get("background-image").unwrap().contains("good.png"));
    }
    #[test]
    fn image_units_keep_percentages_and_unknown_geometry_without_guessing() {
        assert_eq!(computed("radial-gradient(at right 2em bottom 1rem, red 1em, blue 50%)",20.0,30.0),
            "radial-gradient(at calc(100% - 40px) calc(100% - 30px), rgb(255, 0, 0) 20px, rgb(0, 0, 255) 50%)");
        let css=computed("linear-gradient(rgb(1, 2, 3) 2cqw, rgba(1, 2, 3, 0.5) 80%)",20.0,30.0);
        assert!(css.contains("2cqw"));assert!(tokenize(&css).contains(&Token::Number(0.5)));assert!(css.contains("80%"));
        assert_eq!(computed("linear-gradient(red calc(2em + 1rem), blue 50%)",20.0,30.0),
            "linear-gradient(rgb(255, 0, 0) 70px, rgb(0, 0, 255) 50%)");
        assert_eq!(computed("radial-gradient(circle calc(-2em), red -1em, blue)",20.0,30.0),
            "radial-gradient(0px, rgb(255, 0, 0) -20px, rgb(0, 0, 255))");
    }
}
