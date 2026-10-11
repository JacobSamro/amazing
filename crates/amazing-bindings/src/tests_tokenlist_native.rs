use super::*;
#[test]
fn native_token_binding_uses_exact_units_in_value_indices_and_single_old_snapshot() {
    let mut host = Host::default();let node = host.dom.create_element(NodeIdx(0), "div");
    let list = call(&mut host, O::ElementClassList, V::Node(node), &[]);
    let observer = host.dom.create_mutation_observer(1);
    host.dom.observe(observer,node,amazing_dom::mutation::ObserveOptions {attributes:Some(true),attribute_old_value:Some(true),..Default::default()}).unwrap();
    call(&mut host,O::TokenListSetValue,list.clone(),&[V::String(vec![0xd800,32,0xdc00])]);
    call(&mut host,O::TokenListAdd,list.clone(),&[V::String(vec![0xfffd])]);
    assert_eq!(call(&mut host,O::TokenListLength,list.clone(),&[]),V::Number(3.));
    assert_eq!(call(&mut host,O::TokenListItem,list.clone(),&[V::Number(0.)]),V::String(vec![0xd800]));
    assert_eq!(crate::tokenlist::supported_index_count(&mut host,&list).unwrap(),3);
    let records=host.dom.take_mutation_records(observer);
    assert_eq!(records.len(),2);assert_eq!(records[1].old_value_utf16,Some(vec![0xd800,32,0xdc00]));
}
#[test]
fn native_token_binding_rejects_wrong_arena_and_view_generation_before_conversion() {
    let mut host = Host::default();let node=host.dom.create_element(NodeIdx(0),"div");
    let list=call(&mut host,O::ElementClassList,V::Node(node),&[]);
    let V::Platform(key,brand)=list else{panic!("token list key")};
    for key in [PlatformKey{arena:2,..key},PlatformKey{generation:2,..key}] {
        assert!(invoke(&mut host,O::TokenListAdd,V::Platform(key,brand),&[V::Coerce("must not run")]).is_err());
    }
    assert!(host.conversions.is_empty());assert!(host.dom.get_attribute_ns(node,None,"class").is_none());
}
#[test]
fn native_token_binding_supported_policy_uses_actual_namespace_and_attribute() {
    let mut host=Host::default();
    let foreign=host.dom.create_element_ns(NodeIdx(0),Some("urn:foreign"),"a").unwrap();
    let slot=host.dom.token_list(foreign,"rel").unwrap();
    let key=PlatformKey{arena:1,slot:slot as u64,generation:host.dom.token_list_capture(slot).unwrap().view_generation()};
    assert!(invoke(&mut host,O::TokenListSupports,V::Platform(key,InterfaceBrand::DOM_TOKEN_LIST),&[str_value("noopener")]).is_err());
    let node=host.dom.create_element(NodeIdx(0),"div");let class=call(&mut host,O::ElementClassList,V::Node(node),&[]);
    assert!(invoke(&mut host,O::TokenListSupports,class,&[str_value("noopener")]).is_err());
}
#[test]
fn native_token_binding_custom_callbacks_transport_queued_utf16_directly() {
    use amazing_dom::custom_elements::{CustomElementDefinition,LifecycleCallbacks};
    let mut host=Host::default();let node=host.dom.create_element(NodeIdx(0),"x-token-native");
    let callback=host.retain_value(V::Callable(77));
    let mut definition=CustomElementDefinition::new(NodeIdx(0),"x-token-native".into(),"x-token-native".into(),amazing_dom::events::CallbackId(11));
    definition.observed_attributes=vec!["class".into()];
    definition.callbacks=LifecycleCallbacks{attribute_changed:Some(amazing_dom::events::CallbackId(callback.0)),..Default::default()};
    let id=host.dom.define_custom_element(definition).unwrap();host.dom.take_custom_element_reactions();
    host.dom.begin_custom_element_upgrade(node,id).unwrap();host.dom.custom_element_super(id).unwrap();
    host.dom.finish_custom_element_upgrade(node,id,true).unwrap();host.dom.take_custom_element_reactions();
    host.dom.set_attribute_ns_units(node,None,"class",vec![0xd800]).unwrap();
    host.dom.set_attribute_ns_units(node,None,"class",vec![0xdc00]).unwrap();
    host.dom.remove_attribute(node,"class").unwrap();
    crate::deliver_custom_element_reactions(&mut host).unwrap();
    let values:Vec<_>=host.call_receivers.iter().map(|(_,args)|(args[1].clone(),args[2].clone())).collect();
    assert_eq!(values,vec![(V::Null,V::String(vec![0xd800])),(V::String(vec![0xd800]),V::String(vec![0xdc00])),(V::String(vec![0xdc00]),V::Null)]);
    assert_eq!(host.calls,vec![77,77,77]);assert!(host.custom_element_errors.is_empty());
}
