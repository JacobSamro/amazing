use super::*;
use amazing_text::TextSpan;
use crate::installed_font::inspect_installed_font;

fn be16(value: u16) -> [u8;2] { value.to_be_bytes() }
fn be32(value: u32) -> [u8;4] { value.to_be_bytes() }

pub(super) fn metadata_font(weight: u16, width: u16, selection: u16, angle: f32, names: &[(u16,&str)]) -> Vec<u8> {
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

#[test]
fn immutable_installed_handle_shares_exact_bytes_and_metadata_without_reconstruction() {
    let bytes=metadata_font(700,5,1,-12.0,&[(16,"Snapshot Family"),(4,"Snapshot Family Bold Italic"),(6,"SnapshotFamily-BoldItalic")]);
    let face=InstalledFontFace::new(FontFormat::OpenType,bytes.clone()).unwrap();
    let retained=face.clone();
    assert_eq!(face.original_bytes(),bytes);
    assert!(std::ptr::eq(face.profile(),retained.profile()));
    assert_eq!(face.original_bytes().as_ptr(),retained.original_bytes().as_ptr());
    assert_eq!(retained.profile().weight,700);assert_eq!(retained.profile().slant,FontSlant::Italic);
    assert_eq!(retained.profile().postscript_names,["SnapshotFamily-BoldItalic"]);
}

#[test]
fn repeated_installed_registration_shares_physical_storage_preserving_keys_and_budgets() {
    let bytes=metadata_font(400,5,0,0.0,&[(16,"Snapshot Family")]);
    let face=InstalledFontFace::new(FontFormat::OpenType,bytes.clone()).unwrap();
    let mut provider=TextProvider::new();
    let a=provider.register_installed_face_aliases(&["First"],&face).unwrap();
    let b=provider.register_installed_face_aliases(&["Second"],&face.clone()).unwrap();
    assert_ne!(a,b);assert_eq!(provider.fonts.len(),2);
    assert!(std::sync::Arc::ptr_eq(&provider.fonts[0].sfnt,&provider.fonts[1].sfnt));
    assert!(std::sync::Arc::ptr_eq(provider.fonts[0].installed_face.as_ref().unwrap(),provider.fonts[1].installed_face.as_ref().unwrap()));
    assert_eq!(provider.font_set_bytes,bytes.len()*2); // original logical limit
    for (family,key) in [("First",a),("Second",b)] {
        let style=TextStyle{families:vec![family.into()],..TextStyle::default()};
        let span=TextSpan{range:0..1,source:0,style,boundary_before:ShapingBoundary::Continue};
        let shaped=provider.shape(Paragraph{text:"A",spans:&[span],sources:&[],bidi_scopes:&[],base_direction:Direction::LeftToRight}).unwrap();
        assert!(shaped.runs.iter().all(|run|run.font==key));
    }
}

#[test]
fn identical_independent_bytes_share_sfnt_without_merging_native_metadata_or_author_descriptors() {
    let bytes=metadata_font(700,5,1,-12.0,&[(16,"Physical Family")]);
    let mut provider=TextProvider::new();
    let a=provider.register_installed_font("Native",FontFormat::OpenType,bytes.clone()).unwrap();
    let b=provider.register_web_font(WebFont{family:"Author".into(),weight:400,stretch:100,slant:FontSlant::Normal,format:FontFormat::OpenType,bytes}).unwrap();
    assert_ne!(a,b);assert!(std::sync::Arc::ptr_eq(&provider.fonts[0].sfnt,&provider.fonts[1].sfnt));
    assert!(provider.fonts[0].installed_face.is_some());assert!(provider.fonts[1].installed_face.is_none());
    assert_eq!(provider.fonts[0].weight,700);assert_eq!(provider.fonts[1].weight,400);
    assert_eq!(provider.fonts[1].slant,FontSlant::Normal);
}

#[test]
fn cached_handle_does_not_bypass_invalid_aliases_or_existing_face_capacity() {
    let face=InstalledFontFace::new(FontFormat::OpenType,metadata_font(400,5,0,0.0,&[(16,"Real Family")])).unwrap();
    let mut provider=TextProvider::new();
    assert!(matches!(provider.register_installed_face_aliases(&[""],&face),Err(TextError::InvalidStyle)));
    assert!(matches!(provider.register_installed_face_aliases(&["First",""],&face),Err(TextError::InvalidStyle)));
    assert!(provider.fonts.is_empty());assert_eq!(provider.generation(),0);
    for _ in 0..MAX_FONT_FACES { provider.register_installed_face_aliases(&["Real"],&face).unwrap(); }
    let generation=provider.generation();let budget=provider.font_set_bytes;
    assert!(matches!(provider.register_installed_face_aliases(&["Real"],&face),Err(TextError::ResourceLimit)));
    assert_eq!(provider.fonts.len(),MAX_FONT_FACES);assert_eq!(provider.generation(),generation);assert_eq!(provider.font_set_bytes,budget);
}

struct SnapshotLoader { face:InstalledFontFace, byte_calls:usize, url_calls:usize, resource_calls:usize, fail_first:bool }
impl FontFaceLoader for SnapshotLoader {
    fn load_local_face(&mut self,name:&str,_:usize)->Result<Option<InstalledFontFace>,TextError>{
        self.resource_calls+=1;
        if name=="missing" { if self.fail_first { return Err(TextError::MissingFont); }return Ok(None); }
        Ok(Some(self.face.clone()))
    }
    fn load_local(&mut self,_:&str,_:usize)->Result<Option<Vec<u8>>,TextError>{self.byte_calls+=1;Ok(None)}
    fn load_url(&mut self,_:FontFetchRequest<'_>)->Result<Option<Vec<u8>>,TextError>{self.url_calls+=1;Ok(None)}
}
#[test]
fn local_resource_preserves_authored_descriptor_authority_and_source_order() {
    let face=InstalledFontFace::new(FontFormat::OpenType,metadata_font(700,2,1,-12.0,&[(16,"Physical")])).unwrap();
    let mut loader=SnapshotLoader{face,byte_calls:0,url_calls:0,resource_calls:0,fail_first:true};
    let mut provider=TextProvider::new();
    let descriptor=FontFaceDescriptor{family:"Author".into(),sources:vec![FontFaceSource::Local("".into()),FontFaceSource::Local("missing".into()),FontFaceSource::Local("real".into()),FontFaceSource::Url{url:"never".into(),formats:vec![],technologies:vec![]}],weight:FontDescriptorRange::single(400.0),..FontFaceDescriptor::default()};
    let key=provider.register_font_face(descriptor,&mut loader).unwrap().unwrap();
    assert_eq!((loader.resource_calls,loader.byte_calls,loader.url_calls),(2,0,0));
    let font=provider.fonts.iter().find(|f|f.key==key).unwrap();
    assert!(font.installed_face.is_none());assert_eq!(font.weight,400);assert_eq!(font.slant,FontSlant::Normal);
    assert!(std::sync::Arc::ptr_eq(&font.sfnt,&loader.face.sfnt));
    assert_eq!(provider.font_face_registrations()[0].family,"author");
}

#[test]
fn immutable_resource_rejects_malformed_input_and_keeps_cross_provider_ownership() {
    assert!(InstalledFontFace::new(FontFormat::OpenType,b"bad".to_vec()).is_err());
    let face=InstalledFontFace::new(FontFormat::OpenType,metadata_font(400,5,0,0.0,&[(16,"Native")])).unwrap();
    let mut a=TextProvider::new();let mut b=TextProvider::new();
    a.register_installed_face_aliases(&["Native"],&face).unwrap();b.register_installed_face_aliases(&["Native"],&face).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a.fonts[0].sfnt,&b.fonts[0].sfnt));
    drop(a);drop(face);
    assert!(b.matching_font(&TextStyle{families:vec!["Native".into()],..TextStyle::default()},Some("A")).is_some());
}

#[test]
fn cached_collection_keeps_the_original_first_face_policy_and_exact_container_bytes() {
    let bytes=crate::tests::synthetic_ttc('A');
    let face=InstalledFontFace::new(FontFormat::Collection,bytes.clone()).unwrap();
    assert_eq!(face.original_bytes(),bytes);
    let mut provider=TextProvider::new();let key=provider.register_installed_face_aliases(&["Collection"],&face).unwrap();
    assert!(std::sync::Arc::ptr_eq(&provider.fonts[0].sfnt,&face.sfnt));
    let style=TextStyle{families:vec!["Collection".into()],..TextStyle::default()};
    assert_eq!(provider.fonts[provider.matching_font(&style,Some("A")).unwrap()].key,key);
    assert!(provider.matching_font(&style,Some("B")).is_none());
}

#[test]
fn earlier_pending_local_source_does_not_advance_to_later_cached_face() {
    struct PendingLoader { face:InstalledFontFace, calls:Vec<String> }
    impl FontFaceLoader for PendingLoader {
        fn load_local_face(&mut self,name:&str,_:usize)->Result<Option<InstalledFontFace>,TextError>{
            self.calls.push(format!("resource:{name}"));Ok((name=="ready").then(||self.face.clone()))
        }
        fn load_local(&mut self,_:&str,_:usize)->Result<Option<Vec<u8>>,TextError>{panic!("timed source must use native state")}
        fn load_local_state(&mut self,name:&str,_:usize)->Result<FontSourceLoad,TextError>{self.calls.push(format!("state:{name}"));Ok(FontSourceLoad::Pending)}
        fn load_url(&mut self,_:FontFetchRequest<'_>)->Result<Option<Vec<u8>>,TextError>{panic!("later URL must not run")}
    }
    let face=InstalledFontFace::new(FontFormat::OpenType,metadata_font(400,5,0,0.0,&[(16,"Ready")])).unwrap();
    let mut loader=PendingLoader{face,calls:vec![]};let mut provider=TextProvider::new();
    let descriptor=FontFaceDescriptor{family:"Author".into(),sources:vec![FontFaceSource::Local("pending".into()),FontFaceSource::Local("ready".into())],display:FontDisplay::Block,..FontFaceDescriptor::default()};
    let timeline=FontDisplayTimeline::new(descriptor.clone());
    assert!(provider.register_font_face_with_timeline(descriptor,&mut loader,&timeline,std::time::Duration::ZERO).unwrap().is_none());
    assert_eq!(loader.calls,["resource:pending","state:pending"]);assert!(provider.fonts.is_empty());assert_eq!(provider.pending_font_loads(),1);
}
