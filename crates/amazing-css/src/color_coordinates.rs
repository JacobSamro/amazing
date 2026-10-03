//! Floating-point CSS color coordinate conversion; no raster or gradient dispatch.
use super::*;
fn linear_xyz(rgb:[f64;3])->[f64;3] {matrix([
    [0.41239079926595934,0.357584339383878,0.1804807884018343],
    [0.21263900587151027,0.715168678767756,0.07219231536073371],
    [0.01933081871559185,0.11919477979462599,0.9505321522496607]],rgb)}
fn xyz_lab(xyz:[f64;3])->[f64;3] {
    let xyz=matrix(D65_TO_D50,xyz);let white=[0.3457/0.3585,1.0,(1.0-0.3457-0.3585)/0.3585];
    let f=[0,1,2].map(|i|{let n=xyz[i]/white[i];if n>216.0/24389.0 {n.cbrt()}else{(24389.0/27.0*n+16.0)/116.0}});
    [116.0*f[1]-16.0,500.0*(f[0]-f[1]),200.0*(f[1]-f[2])]
}
fn lab_xyz(v:[f64;3])->[f64;3] {
    let y=(v[0]+16.0)/116.0;let f=[y+v[1]/500.0,y,y-v[2]/200.0];
    let white=[0.3457/0.3585,1.0,(1.0-0.3457-0.3585)/0.3585];
    d50_to_d65([0,1,2].map(|i|white[i]*if f[i].powi(3)>216.0/24389.0 {f[i].powi(3)}else{(116.0*f[i]-16.0)/(24389.0/27.0)}))
}
fn xyz_oklab(xyz:[f64;3])->[f64;3] {
    let lms=matrix([[0.819022437996703,0.3619062600528904,-0.1288737815209879],
        [0.0329836539323885,0.9292868615863434,0.0361446663506424],
        [0.0481771893596242,0.2642395317527308,0.6335478284694309]],xyz).map(f64::cbrt);
    matrix([[0.210454268309314,0.7936177747023054,-0.0040720430116193],
        [1.9779985324311684,-2.42859224204858,0.450593709617411],
        [0.0259040424655478,0.7827717124575296,-0.8086757549230774]],lms)
}
fn polar(v:[f64;3])->[f64;3] {[v[0],v[1].hypot(v[2]),v[2].atan2(v[1]).to_degrees().rem_euclid(360.0)]}
fn rectangular(v:[f64;3])->[f64;3] {[v[0],v[1]*v[2].to_radians().cos(),v[1]*v[2].to_radians().sin()]}
fn rgb_hsl(rgb:[f64;3])->[f64;3] {
    let [r,g,b]=rgb;let max=r.max(g).max(b);let min=r.min(g).min(b);let delta=max-min;let d=if delta.abs()<1e-12{0.0}else{delta};let l=(max+min)/2.0;
    let h=if d==0.0 {0.0}else if max==r {((g-b)/d).rem_euclid(6.0)*60.0}else if max==g {((b-r)/d+2.0)*60.0}else{((r-g)/d+4.0)*60.0};
    [h,if d==0.0{0.0}else{d/(1.0-(2.0*l-1.0).abs())*100.0},l*100.0]
}
fn to_xyz(space:&str,v:[f64;3])->Option<[f64;3]> {Some(match space {
    "srgb"=>linear_xyz(v.map(srgb_decode)),"srgb-linear"=>linear_xyz(v),
    "xyz"|"xyz-d65"=>v,"xyz-d50"=>d50_to_d65(v),
    "display-p3"=>matrix(P3_TO_XYZ,v.map(srgb_decode)),"display-p3-linear"=>matrix(P3_TO_XYZ,v),
    "a98-rgb"=>matrix(A98_TO_XYZ,v.map(|n|n.signum()*n.abs().powf(563.0/256.0))),
    "rec2020"=>matrix(REC2020_TO_XYZ,v.map(rec2020_decode)),
    "prophoto-rgb"=>d50_to_d65(matrix(PROPHOTO_TO_XYZ,v.map(|n|if n.abs()<=16.0/512.0 {n/16.0}else{n.signum()*n.abs().powf(1.8)}))),
    "lab"=>lab_xyz(v),"lch"=>lab_xyz(rectangular(v)),"oklab"=>oklab_xyz(v),"oklch"=>oklab_xyz(rectangular(v)),
    "hsl"=>linear_xyz(hsl_rgb(v[0],v[1],v[2]).map(srgb_decode)),
    "hwb"=>linear_xyz(hwb_rgb(v[0],v[1]/100.0,v[2]/100.0).map(srgb_decode)),_=>return None})}
fn from_xyz(space:&str,xyz:[f64;3])->Option<[f64;3]> {Some(match space {
    "srgb"=>xyz_to_srgb(xyz),"srgb-linear"=>xyz_to_srgb(xyz).map(srgb_decode),
    "xyz"|"xyz-d65"=>xyz,"xyz-d50"=>matrix(D65_TO_D50,xyz),
    "display-p3"=>matrix(XYZ_TO_P3,xyz).map(srgb_encode),"display-p3-linear"=>matrix(XYZ_TO_P3,xyz),
    "a98-rgb"=>matrix(XYZ_TO_A98,xyz).map(|n|n.signum()*n.abs().powf(256.0/563.0)),
    "rec2020"=>matrix(XYZ_TO_REC2020,xyz).map(rec2020_encode),
    "prophoto-rgb"=>matrix(XYZ_TO_PROPHOTO,matrix(D65_TO_D50,xyz)).map(|n|if n.abs()<=1.0/512.0{n*16.0}else{n.signum()*n.abs().powf(1.0/1.8)}),
    "lab"=>xyz_lab(xyz),"lch"=>polar(xyz_lab(xyz)),"oklab"=>xyz_oklab(xyz),"oklch"=>polar(xyz_oklab(xyz)),
    "hsl"=>rgb_hsl(xyz_to_srgb(xyz)),"hwb"=>{let rgb=xyz_to_srgb(xyz);[rgb_hsl(rgb)[0],rgb.into_iter().fold(f64::INFINITY,f64::min)*100.0,(1.0-rgb.into_iter().fold(f64::NEG_INFINITY,f64::max))*100.0]},_=>return None})}


pub(crate) fn convert(source:&str,target:&str,channels:[f64;3])->Option<[f64;3]> {
    if source==target {Some(channels)} else {from_xyz(target,to_xyz(source,channels)?)}
}
