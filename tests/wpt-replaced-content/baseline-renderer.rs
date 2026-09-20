//! Static image reftest artifact. Same engine layout/paint APIs as the shell;
//! deterministic fallback fonts, local resources only, no script execution.
use amazing_dom::{Dom,NodeIdx,NodeKind};
use amazing_layout::{IntrinsicSize,ReplacedElementProvider};
use std::{path::{Path,PathBuf},collections::HashMap};
struct Entry { image:amazing_media::Image, size:IntrinsicSize }
struct Resources(HashMap<u32,Entry>);
impl ReplacedElementProvider for Resources {
    fn intrinsic_size(&self,n:NodeIdx)->Option<IntrinsicSize>{self.0.get(&n.0).map(|v|v.size)}
}
impl amazing_paint::ImageProvider for Resources {
    fn image(&self,n:NodeIdx)->Option<&amazing_media::Image>{self.0.get(&n.0).map(|v|&v.image)}
}
fn attr<'a>(dom:&'a Dom,n:NodeIdx,key:&str)->Option<&'a str>{
    dom.attrs_of(n)?.iter().find(|(k,_)|k==key).map(|(_,v)|v.as_str())
}
fn resolve(page:&Path,source:&str)->PathBuf {
    if source.starts_with('/') {
        for parent in page.ancestors() {
            if parent.join(".git").exists() {return parent.join(source.trim_start_matches('/'));}
        }
    }
    page.parent().unwrap().join(source.split('#').next().unwrap_or(source))
}
fn visit(dom:&Dom,n:NodeIdx,f:&mut impl FnMut(NodeIdx)) {
    f(n);for child in dom.children(n){visit(dom,child,f)}
}
fn main()->Result<(),Box<dyn std::error::Error>> {
    let args:Vec<_>=std::env::args().collect();
    if args.len()!=6 || args[1]!="render-wpt" {return Err("expected render-wpt HTML PPM WIDTH HEIGHT".into());}
    let page=Path::new(&args[2]);let html=std::fs::read_to_string(page)?;
    let (width,height):(u32,u32)=(args[4].parse()?,args[5].parse()?);
    if width==0 || height==0 || width>8192 || height>8192 {return Err("viewport limit".into());}
    let dom=amazing_html::parse(&html);let mut css=String::new();
    visit(&dom,NodeIdx(0),&mut |n|{
        if dom.name(n)==Some("style") {
            for c in dom.children(n) {if let NodeKind::Text{data}=&dom.get(c).kind {css.push_str(data);css.push('\n');}}
        } else if dom.name(n)==Some("link") && attr(&dom,n,"rel")==Some("stylesheet") {
            if let Some(href)=attr(&dom,n,"href") {if let Ok(text)=std::fs::read_to_string(resolve(page,href)){css.push_str(&text);css.push('\n');}}
        }
    });
    let mut resources=Resources(HashMap::new());let mut retained=0usize;
    visit(&dom,NodeIdx(0),&mut |n|{
        if dom.name(n)!=Some("img") || resources.0.len()>=256 {return;}
        let Some(src)=attr(&dom,n,"src") else{return;};
        let path=resolve(page,src);
        if std::fs::metadata(&path).map_or(true,|m|m.len()>16*1024*1024){return;}
        let Ok(bytes)=std::fs::read(path) else{return;};
        let image=if std::env::var_os("AMAZING_IMAGE_BASELINE").is_some(){
            amazing_media::decode_image(&bytes).ok()
        }else{amazing_image::decode(&bytes,Default::default()).ok()};
        let Some(image)=image else{return;};
        if retained+image.pixels.len()>128*1024*1024{return;}
        retained+=image.pixels.len();
        let dimension=|name|attr(&dom,n,name).and_then(|s|s.parse::<f32>().ok()).filter(|n|n.is_finite() && *n>=0.0);
        let (w,h)=(dimension("width"),dimension("height"));
        let nw=image.width as f32;let nh=image.height as f32;
        let size=match (w,h){(Some(w),Some(h))=>IntrinsicSize{width:w,height:h},
            (Some(w),None)=>IntrinsicSize{width:w,height:nh*w/nw},
            (None,Some(h))=>IntrinsicSize{width:nw*h/nh,height:h},
            _=>IntrinsicSize{width:nw,height:nh}};
        resources.0.insert(n.0,Entry{image,size});
    });
    let fonts=amazing_layout::font::FontSource::fallback();
    let root=amazing_layout::layout_dom_document_with_replaced_in_viewport(&dom,&css,width as f32,Some(height as f32),&fonts,&resources);
    let glyphs=amazing_paint::RectInkProvider;
    let surface=amazing_paint::paint_document_with_resources(&root,width,height,&fonts,&glyphs,&resources);
    surface.save_ppm(&args[3])?;Ok(())
}
