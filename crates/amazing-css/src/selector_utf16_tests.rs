//! Exact DOMString identity stays distinct from CSS scalar escape normalization.
use super::*;
fn raw(dom:&mut Dom,node:NodeIdx,name:&str,units:&[u16]) {
    dom.set_attribute(node,name,"").unwrap();
    let attr=dom.get_attribute_node(node,None,name).unwrap();
    dom.set_attr_value(attr,units.to_vec()).unwrap();
}
#[test]
fn ids_classes_and_all_attribute_operators_do_not_alias_invalid_code_units() {
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let node=dom.create_element(html,"span");
    for units in [vec![0xd800],vec![0xd83d],vec![0xdc00],vec![0xdfff],vec![0]] {
        for name in ["id","class","data-v"]{raw(&mut dom,node,name,&units);}
        for selector in ["#\\d83d ",".\\d83d ","#\\0 ",".\\0 ","#�",".�"] {
            assert!(!matches(&dom,node,&parse_selector(selector).unwrap()),"{units:?}/{selector}");
        }
        for op in [AttrOp::Equals,AttrOp::Includes,AttrOp::DashMatch,AttrOp::Prefix,AttrOp::Suffix,AttrOp::Substring] {
            for ci in [false,true] {assert!(!match_attr(&dom,node,"data-v",op,"�",ci),"{units:?}/{op:?}");}
        }
        assert!(match_attr(&dom,node,"data-v",AttrOp::Exists,"",false));
    }
    for value in ["�","😀","é","a😀z"] {
        for name in ["id","class","data-v"]{raw(&mut dom,node,name,&value.encode_utf16().collect::<Vec<_>>());}
        assert!(attr_is(&dom,node,"id",value));assert!(class_list_contains(&dom,node,value));
        for op in [AttrOp::Equals,AttrOp::Includes,AttrOp::DashMatch,AttrOp::Prefix,AttrOp::Suffix,AttrOp::Substring] {
            assert!(match_attr(&dom,node,"data-v",op,value,false),"{value}/{op:?}");
        }
    }
}
#[test]
fn operators_preserve_ascii_case_space_empty_and_substring_boundaries() {
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let node=dom.create_element(html,"span");
    raw(&mut dom,node,"data-v",&[0x41,0xd800,0x5a]);
    assert!(match_attr(&dom,node,"data-v",AttrOp::Prefix,"a",true));
    assert!(match_attr(&dom,node,"data-v",AttrOp::Suffix,"z",true));
    assert!(!match_attr(&dom,node,"data-v",AttrOp::Substring,"a�z",true));
    for space in [0x09,0x0a,0x0c,0x0d,0x20] {
        raw(&mut dom,node,"class",&[0x41,space,0x42]);raw(&mut dom,node,"data-v",&[0x41,space,0x42]);
        assert!(class_list_contains(&dom,node,"A"));assert!(class_list_contains(&dom,node,"B"));
        assert!(match_attr(&dom,node,"data-v",AttrOp::Includes,"b",true));
    }
    for space in [0x0b,0x85,0xa0] {
        raw(&mut dom,node,"class",&[0x41,space,0x42]);raw(&mut dom,node,"data-v",&[0x41,space,0x42]);
        assert!(!class_list_contains(&dom,node,"A"));assert!(!match_attr(&dom,node,"data-v",AttrOp::Includes,"A",false));
    }
    raw(&mut dom,node,"data-v",&"É-foo".encode_utf16().collect::<Vec<_>>());
    assert!(!match_attr(&dom,node,"data-v",AttrOp::Prefix,"é",true));
    assert!(match_attr(&dom,node,"data-v",AttrOp::DashMatch,"É",true));
    for op in [AttrOp::Includes,AttrOp::Prefix,AttrOp::Suffix,AttrOp::Substring] {
        assert!(!match_attr(&dom,node,"data-v",op,"",false));
    }
    raw(&mut dom,node,"data-v",&[]);assert!(match_attr(&dom,node,"data-v",AttrOp::Equals,"",false));
    assert!(!match_attr(&dom,node,"absent",AttrOp::Exists,"",false));
}
