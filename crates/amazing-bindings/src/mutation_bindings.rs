use crate::host::{BindingHost, CallbackId, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{
    mutation::{MutationRecordId, MutationType, ObserveOptions, ObserverId},
    DomError, NodeIdx,
};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::MutationObserverConstructor | O::MutationObserve | O::MutationDisconnect
        | O::MutationTakeRecords | O::MutationRecordType | O::MutationRecordTarget
        | O::MutationRecordAddedNodes | O::MutationRecordRemovedNodes
        | O::MutationRecordPreviousSibling | O::MutationRecordNextSibling
        | O::MutationRecordAttributeName | O::MutationRecordAttributeNamespace
        | O::MutationRecordOldValue)
}
fn key<H: BindingHost>(host: &H, slot: usize) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
        slot: slot as u64, generation: 1 }
}
fn observer<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<ObserverId, H::Error> {
    let key = host.platform_object(value, InterfaceBrand::MUTATION_OBSERVER)
        .ok_or_else(|| host.type_error("Expected a MutationObserver"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid MutationObserver"));
    }
    let id = ObserverId::from_slot(key.slot as usize);
    host.with_dom(|dom| dom.observer_callback(id).ok_or(DomError::InvalidStateError).map(|_| ()))?;
    Ok(id)
}
fn record<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<MutationRecordId, H::Error> {
    let key = host.platform_object(value, InterfaceBrand::MUTATION_RECORD)
        .ok_or_else(|| host.type_error("Expected a MutationRecord"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid MutationRecord"));
    }
    let id = MutationRecordId::from_slot(key.slot as usize);
    host.with_dom(|dom| dom.mutation_record(id).ok_or(DomError::InvalidStateError).map(|_| ()))?;
    Ok(id)
}
fn field<H: BindingHost>(host: &mut H, dictionary: &H::Value, name: &str) -> Result<H::Value, H::Error> {
    host.get_property(dictionary, name)
}
fn boolean<H: BindingHost>(host: &mut H, dictionary: &H::Value, name: &str) -> Result<Option<bool>, H::Error> {
    let value = field(host, dictionary, name)?;
    Ok((!host.is_undefined(&value)).then(|| host.to_boolean(&value)))
}
fn options<H: BindingHost>(host: &mut H, dictionary: &H::Value) -> Result<ObserveOptions, H::Error> {
    let child_list = boolean(host, dictionary, "childList")?.unwrap_or(false);
    let attributes = boolean(host, dictionary, "attributes")?;
    let character_data = boolean(host, dictionary, "characterData")?;
    let subtree = boolean(host, dictionary, "subtree")?.unwrap_or(false);
    let attribute_old_value = boolean(host, dictionary, "attributeOldValue")?;
    let character_data_old_value = boolean(host, dictionary, "characterDataOldValue")?;
    let filter = field(host, dictionary, "attributeFilter")?;
    let attribute_filter = if host.is_undefined(&filter) { None } else {
        let length_value = host.get_property(&filter, "length")?;
        let length = host.to_number(&length_value)?;
        if !length.is_finite() || length < 0. || length > 100_000. {
            return Err(host.type_error("Invalid attributeFilter length"));
        }
        let mut names = Vec::new();
        for index in 0..length.trunc() as usize {
            let value = host.get_property(&filter, &index.to_string())?;
            let units = host.to_dom_string(&value)?;
            names.push(String::from_utf16(&units).map_err(|_| host.type_error("Invalid attributeFilter name"))?);
        }
        Some(names)
    };
    Ok(ObserveOptions {
        child_list, attributes, character_data, subtree,
        attribute_old_value, character_data_old_value, attribute_filter,
    })
}
fn wrap_records<H: BindingHost>(host: &mut H, ids: Vec<MutationRecordId>) -> Result<H::Value, H::Error> {
    let mut values = Vec::with_capacity(ids.len());
    for id in ids {
        values.push(host.wrap_platform_object(key(host, id.slot()), InterfaceBrand::MUTATION_RECORD)?);
    }
    host.create_array(values)
}
fn wrap_optional_node<H: BindingHost>(host: &mut H, node: Option<NodeIdx>) -> Result<H::Value, H::Error> {
    match node { Some(node) => host.wrap_node(node), None => Ok(host.null()) }
}
enum Output { Text(Option<Vec<u16>>), Node(Option<NodeIdx>), List(usize) }

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if op == O::MutationObserverConstructor {
        let callback = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        if !host.is_callable(callback) { return Err(host.type_error("MutationCallback must be callable")); }
        let token = host.retain_value(callback.clone());
        let id = host.with_dom(|dom| Ok(dom.create_mutation_observer(token.0)))?;
        return host.wrap_platform_object(key(host, id.slot()), InterfaceBrand::MUTATION_OBSERVER);
    }
    if matches!(op, O::MutationObserve | O::MutationDisconnect | O::MutationTakeRecords) {
        let id = observer(host, &receiver)?;
        return match op {
            O::MutationObserve => {
                let target = args.first().and_then(|value| host.node_brand(value))
                    .ok_or_else(|| host.type_error("Expected a Node"))?;
                let dictionary = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
                let options = options(host, dictionary)?;
                host.with_dom(|dom| dom.observe(id, target, options))?;
                Ok(host.undefined())
            }
            O::MutationDisconnect => {
                host.with_dom(|dom| { dom.disconnect_observer(id); Ok(()) })?;
                // The callback stays retained: observe() may be called again.
                Ok(host.undefined())
            }
            O::MutationTakeRecords => {
                let ids = host.with_dom(|dom| {
                    let records = dom.take_mutation_records(id);
                    Ok(records.into_iter().map(|record| dom.register_mutation_record(record)).collect())
                })?;
                wrap_records(host, ids)
            }
            _ => unreachable!(),
        };
    }
    let id = record(host, &receiver)?;
    let output = host.with_dom(|dom| {
        let record = dom.mutation_record(id).ok_or(DomError::InvalidStateError)?;
        let result = match op {
            O::MutationRecordType => Output::Text(Some(match record.kind {
                MutationType::ChildList => "childList", MutationType::Attributes => "attributes",
                MutationType::CharacterData => "characterData",
            }.encode_utf16().collect())),
            O::MutationRecordTarget => Output::Node(Some(record.target)),
            O::MutationRecordPreviousSibling => Output::Node(record.previous_sibling),
            O::MutationRecordNextSibling => Output::Node(record.next_sibling),
            O::MutationRecordAttributeName => Output::Text(record.attribute_name.as_ref().map(|s| s.encode_utf16().collect())),
            O::MutationRecordAttributeNamespace => Output::Text(record.attribute_namespace.as_ref().map(|s| s.encode_utf16().collect())),
            O::MutationRecordOldValue => Output::Text(record.old_value_utf16.clone().or_else(|| record.old_value.as_ref().map(|s| s.encode_utf16().collect()))),
            O::MutationRecordAddedNodes | O::MutationRecordRemovedNodes => {
                let (added, removed) = dom.mutation_record_lists(id).ok_or(DomError::InvalidStateError)?;
                Output::List(if op == O::MutationRecordAddedNodes { added } else { removed })
            }
            _ => unreachable!(),
        };
        Ok(result)
    })?;
    match output {
        Output::Text(Some(units)) => host.string(&units),
        Output::Text(None) => Ok(host.null()),
        Output::Node(node) => wrap_optional_node(host, node),
        Output::List(slot) => host.wrap_platform_object(key(host, slot), InterfaceBrand::STATIC_NODE_LIST),
    }
}

/// Called by the driver for the JS-4 MutationObserver native microtask step.
/// Each callback runs after releasing the DOM borrow. A thrown callback is
/// remembered while later observers still receive their records.
pub fn deliver_mutation_checkpoint<H: BindingHost>(host: &mut H) -> Result<(), H::Error> {
    let mut checkpoint = host.with_dom(|dom| Ok(dom.begin_mutation_checkpoint()))?;
    let mut first_error = None;
    loop {
        let delivery = host.with_dom(|dom| {
            let Some(delivery) = dom.next_mutation_delivery(&mut checkpoint) else { return Ok(None) };
            let ids = delivery.records.into_iter().map(|record| dom.register_mutation_record(record)).collect();
            Ok(Some((delivery.observer, delivery.callback, ids)))
        })?;
        let Some((observer, callback, records)) = delivery else { break };
        let values = wrap_records(host, records)?;
        let observer_value = host.wrap_platform_object(key(host, observer.slot()), InterfaceBrand::MUTATION_OBSERVER)?;
        let function = host.root_value(CallbackId(callback))
            .ok_or_else(|| host.type_error("Released MutationCallback"))?;
        let this = host.undefined();
        if let Err(error) = host.call(&function, &this, &[values, observer_value]) {
            if first_error.is_none() { first_error = Some(error); }
        }
    }
    match first_error { Some(error) => Err(error), None => Ok(()) }
}
