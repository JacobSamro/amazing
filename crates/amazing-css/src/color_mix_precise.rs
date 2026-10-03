//! Precision-preserving computed two-color mixes; independent of raster dispatch.
use super::*;
impl ColorValue {
    pub(super) fn mix(inner:&str,current:Option<&Self>,dark:bool,depth:usize)->Option<Self> {
        let groups=words(inner,',')?;let [method,one,two]=groups.as_slice() else{return None};
        let method=method.trim().strip_prefix("in ")?;let method=advanced::top_level_words(method);
        let space=*method.first()?;
        let polar=matches!(space,"hsl"|"hwb"|"lch"|"oklch");
        let hue_method=match method.as_slice() {[_]=>"shorter",[_,mode,"hue"] if polar && matches!(*mode,"shorter"|"longer"|"increasing"|"decreasing")=>*mode,_=>return None};
        let item=|css:&str|->Option<(Self,Option<f64>)> {
            let tokens=tokenize(css);let mut parts=crate::computed::top_level_components(&tokens)?;
            let weight=if let Some([Token::Percentage(n)])=parts.last().copied() {
                if !n.is_finite() || *n<0.0 || *n>100.0 {return None;} let n=*n/100.0;parts.pop();Some(n)
            } else {None};
            if parts.len()!=1 {return None;}
            Some((Self::resolve_depth(&crate::syntax::serialize(parts[0]),current,dark,depth)?,weight))
        };
        let (one,p)=item(one)?;let (two,q)=item(two)?;
        let (p,q)=match (p,q) {(None,None)=>(0.5,0.5),(Some(p),None)=>(p,1.0-p),(None,Some(q))=>(1.0-q,q),(Some(p),Some(q))=>(p,q)};
        let total=p+q;if total<=0.0 {return None;} let t=q/total;
        let (mut a,ma)=one.convert_components(space)?;let (mut b,mb)=two.convert_components(space)?;
        for i in 0..3 {match (ma[i],mb[i]) {(true,true)=>{a[i]=0.0;b[i]=0.0},(true,false)=>a[i]=b[i],(false,true)=>b[i]=a[i],_=>{}}}
        let aa=one.alpha.or(two.alpha).unwrap_or(1.0);let ba=two.alpha.or(one.alpha).unwrap_or(1.0);
        let alpha=aa*(1.0-t)+ba*t;
        let hue=match space {"hsl"|"hwb"=>Some(0),"lch"|"oklch"=>Some(2),_=>None};
        let channels=[0,1,2].map(|i| {
            if ma[i] && mb[i] {None} else {Some(if hue==Some(i) {hue_mix(a[i],b[i],t,hue_method)}
                else if alpha>0.0 {(a[i]*aa*(1.0-t)+b[i]*ba*t)/alpha} else {0.0})}
        });
        Some(Self{space:space.into(),channels,alpha:if one.alpha.is_none()&&two.alpha.is_none(){None}else{Some(alpha*total.min(1.0))},legacy:false})
    }
    pub(super) fn convert_components(&self,target:&str)->Option<([f64;3],[bool;3])> {
        let source=self.channels.map(|n|n.unwrap_or(0.0));
        let channels=advanced::coordinates::convert(&self.space,target,source)?;
        let category=|space:&str|match space {
            "srgb"|"srgb-linear"|"display-p3"|"display-p3-linear"|"a98-rgb"|"prophoto-rgb"|"rec2020"|"xyz"|"xyz-d65"|"xyz-d50"=>[0,1,2],
            "lab"|"oklab"=>[3,4,5],"lch"|"oklch"=>[3,6,7],"hsl"=>[7,6,3],"hwb"=>[7,8,9],_=>[10,11,12]};
        let input=category(&self.space);let output=category(target);
        let mut missing=[0,1,2].map(|i|(0..3).any(|j|input[j]==output[i]&&self.channels[j].is_none()));
        let source_set:Vec<_>=(0..3).filter(|&i|!output.contains(&input[i])).collect();
        if !source_set.is_empty()&&source_set.iter().all(|&i|self.channels[i].is_none()) {
            for i in 0..3 {if !input.contains(&output[i]) {missing[i]=true;}}
        }
        if self.space!=target {
            let hue=match target {"hsl"|"hwb"=>Some(0),"lch"|"oklch"=>Some(2),_=>None};
            if let Some(h)=hue {let powerless=if target=="hwb" {channels[1]+channels[2]>=100.0}else{channels[1].abs()<1e-7};if powerless {missing[h]=true;}}
        }
        Some((channels,missing))
    }
}
fn hue_mix(mut one:f64,mut two:f64,t:f64,method:&str)->f64 {
    let difference=two-one;
    match method {
        "shorter"=>if difference>180.0 {one+=360.0}else if difference< -180.0 {two+=360.0},
        "longer"=>if difference>0.0&&difference<180.0 {one+=360.0}else if difference> -180.0&&difference<=0.0 {two+=360.0},
        "increasing"=>if two<one {two+=360.0},"decreasing"=>if one<two {one+=360.0},_=>{},
    }
    (one+(two-one)*t).rem_euclid(360.0)
}
