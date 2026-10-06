use super::*;
use amazing_text::TextSpan;
use crate::installed_font::inspect_installed_font;

fn be16(value: u16) -> [u8;2] { value.to_be_bytes() }
fn be32(value: u32) -> [u8;4] { value.to_be_bytes() }

fn metadata_font(weight: u16, width: u16, selection: u16, angle: f32, names: &[(u16,&str)]) -> Vec<u8> {
    let original = crate::tests::synthetic_font('A');
    let count = u16::from_be_bytes(original[4..6].try_into().unwrap()) as usize;
    let mut tables: Vec<([u8;4],Vec<u8>)> = (0..count).map(|i| {
        let row = &original[12+i*16..28+i*16];
        let offset = u32::from_be_bytes(row[8..12].try_into().unwrap()) as usize;
        let size = u32::from_be_bytes(row[12..16].try_into().unwrap()) as usize;
        (row[..4].try_into().unwrap(), original[offset..offset+size].to_vec())
    }).collect();
    let mut os2 = vec![0;96]; os2[..2].copy_from_slice(&be16(4));
    os2[4..6].copy_from_slice(&be16(weight)); os2[6..8].copy_from_slice(&be16(width));
    os2[30..32].copy_from_slice(&be16(0x0100));
    os2[32]=2;os2[33]=2;os2[62..64].copy_from_slice(&be16(selection));
    let mut post = vec![0;32];post[..4].copy_from_slice(&be32(0x00030000));
    post[4..8].copy_from_slice(&((angle*65536.0) as i32).to_be_bytes());
    let mut name = [be16(0),be16(names.len() as u16),be16(6+names.len() as u16*12)].concat();
    let mut strings = Vec::new();
    for (id,value) in names {
        let encoded = value.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<_>>();
        name.extend_from_slice(&[be16(3),be16(1),be16(0x409),be16(*id),be16(encoded.len() as u16),be16(strings.len() as u16)].concat());
        strings.extend(encoded);
    }
    name.extend(strings);
    tables.extend([(*b"OS/2",os2),(*b"post",post),(*b"name",name)]);
    let refs=tables.iter().map(|(tag,data)|(tag,data.clone())).collect::<Vec<_>>();
    crate::tests::assemble_sfnt(&refs)
}

#[test]
fn actual_native_names_and_classification_survive_without_filename_aliases() {
    let bytes=metadata_font(700,2,0,-12.0,&[(1,"Legacy Bold"),(16,"Actual Family"),(21,"WWS Family"),(4,"Actual Family Bold"),(6,"ActualFamily-Bold"),(1,"中文字体")]);
    let p=inspect_installed_font(FontFormat::OpenType,&bytes).unwrap();
    assert_eq!(p.families,["WWS Family","Actual Family","Legacy Bold","中文字体"]);
    assert_eq!(p.full_names,["Actual Family Bold"]);assert_eq!(p.postscript_names,["ActualFamily-Bold"]);
    assert_eq!(p.weight,700);assert_eq!(p.stretch,62.5);assert_eq!(p.serif,Some(true));
    assert_eq!(p.italic_angle,Some(-12.0));assert!(!p.fixed_pitch);
}

#[test]
fn all_os2_width_classes_and_weights_preserve_native_values() {
    for (width,percentage) in [50.0,62.5,75.0,87.5,100.0,112.5,125.0,150.0,200.0].into_iter().enumerate() {
        for weight in [1,100,400,700,1000] {
            let p=inspect_installed_font(FontFormat::OpenType,&metadata_font(weight,width as u16+1,0,0.0,&[])).unwrap();
            assert_eq!(p.weight,weight);assert_eq!(p.stretch,percentage);
        }
    }
    let p=inspect_installed_font(FontFormat::OpenType,&metadata_font(0,0,0,0.0,&[])).unwrap();
    assert_eq!(p.weight,400);assert_eq!(p.stretch,100.0);
}

#[test]
fn actual_italic_and_oblique_flags_keep_css_angle_sign_and_fractional_matching() {
    let mut provider=TextProvider::new();
    let normal=provider.register_installed_font("Style",FontFormat::OpenType,metadata_font(400,2,0,0.0,&[])).unwrap();
    let italic=provider.register_installed_font("Style",FontFormat::OpenType,metadata_font(400,2,1,-10.0,&[])).unwrap();
    let oblique=provider.register_installed_font("Style",FontFormat::OpenType,metadata_font(400,2,0x200,-22.0,&[])).unwrap();
    let mut style=TextStyle {families:vec!["Style".into()],stretch_percent:62.5,..TextStyle::default()};
    for (slant,angle,key) in [(FontSlant::Normal,0.0,normal),(FontSlant::Italic,0.0,italic),(FontSlant::Oblique,22.0,oblique)] {
        style.slant=slant;style.oblique_angle_degrees=Some(angle);
        let index=provider.matching_font(&style,Some("A")).unwrap();assert_eq!(provider.fonts[index].key,key);
    }
}

#[test]
fn real_bold_style_selection_does_not_relabel_the_regular_font() {
    let mut provider=TextProvider::new();
    let regular=provider.register_installed_font("Family",FontFormat::OpenType,metadata_font(400,5,0,0.0,&[])).unwrap();
    let bold=provider.register_installed_font("Family",FontFormat::OpenType,metadata_font(700,5,0,0.0,&[])).unwrap();
    for (weight,key) in [(400,regular),(700,bold)] {
        let style=TextStyle {families:vec!["Family".into()],weight,synthesis:FontSynthesis {weight:false,..FontSynthesis::default()},..TextStyle::default()};
        let index=provider.matching_font(&style,Some("A")).unwrap();assert_eq!(provider.fonts[index].key,key);
        let spans=[TextSpan {range:0..1,source:0,style,boundary_before:ShapingBoundary::Continue}];
        let paragraph=Paragraph {text:"A",spans:&spans,sources:&[],bidi_scopes:&[],base_direction:Direction::LeftToRight};
        let shaped=provider.shape(paragraph).unwrap();assert!(!shaped.runs.is_empty());assert!(shaped.runs.iter().all(|run|run.font==key));
    }
}

#[test]
fn authored_descriptors_remain_distinct_from_installed_native_metadata() {
    let bytes=metadata_font(700,2,1,-12.0,&[]);let mut p=TextProvider::new();
    let authored=p.register_web_font(WebFont {family:"Author".into(),weight:400,stretch:100,slant:FontSlant::Normal,format:FontFormat::OpenType,bytes:bytes.clone()}).unwrap();
    let installed=p.register_installed_font("Installed",FontFormat::OpenType,bytes).unwrap();
    let a=p.fonts.iter().find(|f|f.key==authored).unwrap();assert!(a.installed_face.is_none());assert_eq!(a.weight,400);
    let i=p.fonts.iter().find(|f|f.key==installed).unwrap();assert_eq!(i.weight,700);assert_eq!(i.installed_face.as_ref().unwrap().stretch,62.5);
}

#[test]
fn malformed_fonts_and_missing_name_metadata_do_not_invent_identity() {
    assert!(inspect_installed_font(FontFormat::OpenType,b"not a font").is_err());
    let p=inspect_installed_font(FontFormat::OpenType,&crate::tests::synthetic_font('A')).unwrap();
    assert!(p.families.is_empty());assert!(p.full_names.is_empty());assert!(p.postscript_names.is_empty());
    assert_eq!(p.weight,400);assert_eq!(p.family_class,None);assert_eq!(p.serif,None);
}

fn add_table(original: &[u8], added_tag: [u8;4], data: Vec<u8>) -> Vec<u8> {
    let count=u16::from_be_bytes(original[4..6].try_into().unwrap()) as usize;
    let mut tables:Vec<([u8;4],Vec<u8>)>=(0..count).map(|i| {
        let row=&original[12+i*16..28+i*16];let offset=u32::from_be_bytes(row[8..12].try_into().unwrap()) as usize;
        let size=u32::from_be_bytes(row[12..16].try_into().unwrap()) as usize;
        (row[..4].try_into().unwrap(),original[offset..offset+size].to_vec())
    }).filter(|(tag,_)|*tag!=added_tag).collect();tables.push((added_tag,data));
    crate::tests::assemble_sfnt(&tables.iter().map(|(tag,data)|(tag,data.clone())).collect::<Vec<_>>())
}

#[test]
fn installed_variable_axes_reach_matching_shaping_and_synthesis_without_recreation() {
    let mut fvar=[be16(1),be16(0),be16(16),be16(2),be16(4),be16(20),be16(0),be16(20)].concat();
    for (tag,min,default,max) in [(*b"wght",200.0,400.0,900.0),(*b"wdth",62.5,100.0,125.0),(*b"slnt",-30.0,0.0,0.0),(*b"ital",0.0,0.0,1.0)] {
        fvar.extend(tag);for value in [min,default,max] { fvar.extend(((value*65536.0) as i32).to_be_bytes()); }
        fvar.extend([be16(0),be16(256)].concat());
    }
    let bytes=add_table(&metadata_font(400,5,0,0.0,&[(1,"Variable")]),*b"fvar",fvar);
    let metadata=inspect_installed_font(FontFormat::OpenType,&bytes).unwrap();
    assert_eq!(metadata.weight_range,FontDescriptorRange {min:200.0,max:900.0});
    assert_eq!(metadata.stretch_range,FontDescriptorRange {min:62.5,max:125.0});
    let mut provider=TextProvider::new();let key=provider.register_installed_font("Variable",FontFormat::OpenType,bytes).unwrap();
    let style=TextStyle {families:vec!["Variable".into()],weight:800,stretch_percent:62.5,slant:FontSlant::Italic,..TextStyle::default()};
    let font=&provider.fonts[provider.matching_font(&style,Some("A")).unwrap()];assert_eq!(font.key,key);
    assert_eq!(font.attributes_for(&style).weight,800);assert_eq!(font.attributes_for(&style).slant,FontSlant::Italic);
    let coordinates=font_variations::for_face(font,&style);
    for (tag,value) in [(*b"wght",800.0),(*b"wdth",62.5),(*b"ital",1.0)] {
        assert_eq!(coordinates.iter().find(|axis|axis.tag==tag).unwrap().value,value);
    }
    assert!(!coordinates.iter().any(|axis|axis.tag==*b"slnt"));
    let oblique_style=TextStyle {slant:FontSlant::Oblique,oblique_angle_degrees:Some(22.0),..style.clone()};
    assert_eq!(font.attributes_for(&oblique_style).slant,FontSlant::Oblique);
    let oblique=font_variations::for_face(font,&oblique_style);
    assert_eq!(oblique.iter().find(|axis|axis.tag==*b"ital").unwrap().value,0.0);
    assert_eq!(oblique.iter().find(|axis|axis.tag==*b"slnt").unwrap().value,-22.0);
    let spans=[TextSpan {range:0..1,source:0,style,boundary_before:ShapingBoundary::Continue}];
    let shaped=provider.shape(Paragraph {text:"A",spans:&spans,sources:&[],bidi_scopes:&[],base_direction:Direction::LeftToRight}).unwrap();
    assert!(shaped.runs.iter().all(|run| !run.synthesis.weight && !run.synthesis.style));
}

#[test]
fn named_metadata_normalization_rejects_empty_nul_and_deduplicates_real_aliases() {
    let bytes=metadata_font(400,5,0,0.0,&[(21,"Same Family"),(16,"same family"),(1," "),(1,"bad\0identity"),(4,"Full Face"),(6,"FullFace")]);
    let p=inspect_installed_font(FontFormat::OpenType,&bytes).unwrap();
    assert_eq!(p.families,["Same Family"]);assert_eq!(p.full_names,["Full Face"]);assert_eq!(p.postscript_names,["FullFace"]);
    let mut bad=bytes.clone();bad.truncate(15);assert!(inspect_installed_font(FontFormat::OpenType,&bad).is_err());
    // Ill-formed UTF16 name storage never creates a replacement-character
    // alias, while a genuine U+FFFD name remains its actual native identity.
    for (unit, expected) in [(0xD800,Vec::<String>::new()),(0xFFFD,vec!["\u{FFFD}".to_owned()])] {
        let name=[be16(0),be16(1),be16(18),be16(3),be16(1),be16(0x409),be16(1),be16(2),be16(0),be16(unit)].concat();
        let file=add_table(&metadata_font(400,5,0,0.0,&[]),*b"name",name);
        assert_eq!(inspect_installed_font(FontFormat::OpenType,&file).unwrap().families,expected);
    }
}

#[test]
fn native_face_aliases_share_actual_bytes_keys_and_family_selection() {
    let bytes=metadata_font(700,5,0,0.0,&[(16,"Actual Serif"),(1,"Legacy Serif Bold")]);
    let mut p=TextProvider::new();let key=p.register_installed_font_aliases(&["serif","ui-serif","Times"],FontFormat::OpenType,bytes).unwrap();
    assert_eq!(p.fonts.len(),1);let native_bytes=p.font_set_bytes;
    for family in ["serif","ui-serif","Times","Actual Serif","Legacy Serif Bold"] {
        let style=TextStyle {families:vec![family.into()],weight:700,..TextStyle::default()};
        assert_eq!(p.fonts[p.matching_font(&style,Some("A")).unwrap()].key,key);
    }
    assert_eq!(p.font_set_bytes,native_bytes);
    let count=p.fonts.len();assert!(p.register_installed_font_aliases(&["serif",""],FontFormat::OpenType,metadata_font(400,5,0,0.0,&[])).is_err());assert_eq!(p.fonts.len(),count);
}
