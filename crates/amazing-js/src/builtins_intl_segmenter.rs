// Intl.Segmenter with Unicode-aware UTF-16 indices.

#[derive(Clone)]
struct IntlSegmenterState { locale: String, resolved: String, granularity: String }

#[derive(Clone)]
struct IntlSegment { text: String, index: usize, end: usize, word_like: bool }

#[derive(Clone)]
struct IntlSegmentsState { input: String, granularity: String, pieces: Vec<IntlSegment> }

#[derive(Clone)]
struct IntlSegmentIteratorState { input: String, granularity: String, pieces: Vec<IntlSegment>, cursor: Rc<RefCell<usize>> }

impl Interpreter {
    fn install_intl_segmenter(&mut self, namespace: &Obj) {
        let constructor = Self::native("Segmenter", intl_segmenter_ctor, &self.function_proto);
        let JsValue::Object(function) = &constructor else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "Segmenter", 0, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(constructor.clone()));
        intl_method(self, &prototype, "segment", 1, intl_segmenter_segment);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_segmenter_resolved_options);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.Segmenter")));
        let _ = namespace.borrow_mut().define("Segmenter", binary_writable_property(constructor));
    }
}

fn intl_segmenter_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("Segmenter requires new")); }
    let JsValue::Object(object) = this else { return Err(Throw::type_error("Segmenter requires an object")); };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let locale = intl_lookup(&requested);
    let options = args.get(1).filter(|value| !value.is_undefined()).cloned().unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null Segmenter options")); }
    let _matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let granularity = intl_string_option(i, &options, "granularity", &["grapheme", "word", "sentence"], "grapheme")?;
    let resolved = intl_resolved_locale(&requested, &locale);
    intl_register(&object, IntlState::Segmenter(IntlSegmenterState { locale, resolved, granularity }));
    Ok(JsValue::Object(object))
}

fn intl_segmenter_state(this: &JsValue) -> Result<IntlSegmenterState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible Segmenter receiver")); };
    match intl_state(object) {
        Some(IntlState::Segmenter(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible Segmenter receiver")),
    }
}

fn intl_segmenter_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_segmenter_state(&this)?;
    let object = i.object_value();
    i.set_prop(&object, "locale", JsValue::str(state.resolved.as_str()))?;
    i.set_prop(&object, "granularity", JsValue::str(state.granularity))?;
    Ok(object)
}

fn intl_segmenter_segment(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_segmenter_state(&this)?;
    let input = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let pieces = intl_make_segments(&input, &state.granularity);
    let mut raw = JsObject::ordinary();
    raw.proto = Some(i.object_proto.clone());
    let object = Obj(Rc::new(RefCell::new(raw)));
    intl_method(i, &object, "containing", 1, intl_segments_containing);
    let iterator = Interpreter::native("[Symbol.iterator]", intl_segments_iterator, &i.function_proto);
    object.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), binary_writable_property(iterator)).unwrap();
    intl_register(&object, IntlState::Segments(IntlSegmentsState { input, granularity: state.granularity, pieces }));
    Ok(JsValue::Object(object))
}

fn intl_is_extend(ch: char) -> bool {
    matches!(ch as u32,
        0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff |
        0x20d0..=0x20ff | 0xfe20..=0xfe2f | 0xfe00..=0xfe0f |
        0x1f3fb..=0x1f3ff)
}

fn intl_make_segments(input: &str, granularity: &str) -> Vec<IntlSegment> {
    let mut pieces = Vec::new();
    let mut start_byte = 0;
    let mut start_utf16 = 0;
    let mut current_utf16 = 0;
    let mut previous = None;
    let mut regional_count = 0;
    let mut sentence_end = false;
    let mut word_kind = None;
    let mut chars = input.char_indices().peekable();
    while let Some((byte, ch)) = chars.next() {
        let is_word = ch.is_alphanumeric() || ch == '_';
        let boundary = if byte == 0 { false } else {
            match granularity {
                "word" => word_kind.is_some_and(|old| old != is_word)
                    || (!is_word && previous.is_some_and(|old: char| old.is_whitespace() != ch.is_whitespace())),
                "sentence" => sentence_end && !ch.is_whitespace(),
                _ => {
                    let previous = previous.unwrap();
                    !(intl_is_extend(ch) || ch == '\u{200d}' || previous == '\u{200d}'
                        || (previous == '\r' && ch == '\n')
                        || (matches!(previous as u32, 0x1f1e6..=0x1f1ff)
                            && matches!(ch as u32, 0x1f1e6..=0x1f1ff) && regional_count % 2 == 1))
                }
            }
        };
        if boundary {
            let text = input[start_byte..byte].to_string();
            pieces.push(IntlSegment {
                text, index: start_utf16, end: current_utf16,
                word_like: word_kind.unwrap_or(false),
            });
            start_byte = byte;
            start_utf16 = current_utf16;
        }
        current_utf16 += ch.len_utf16();
        if matches!(ch as u32, 0x1f1e6..=0x1f1ff) { regional_count += 1; } else { regional_count = 0; }
        if granularity == "sentence" {
            if ['.', '!', '?', '。', '！', '？'].contains(&ch) { sentence_end = true; }
            else if !ch.is_whitespace() { sentence_end = false; }
        }
        if granularity == "word" { word_kind = Some(is_word); }
        previous = Some(ch);
    }
    if start_byte < input.len() {
        pieces.push(IntlSegment {
            text: input[start_byte..].to_string(), index: start_utf16,
            end: current_utf16, word_like: word_kind.unwrap_or(false),
        });
    }
    pieces
}

fn intl_segment_record(i: &mut Interpreter, segment: &IntlSegment, input: &str, include_word_like: bool) -> Result<JsValue, Throw> {
    let object = i.object_value();
    i.set_prop(&object, "segment", JsValue::str(&segment.text))?;
    i.set_prop(&object, "index", JsValue::num(segment.index as f64))?;
    i.set_prop(&object, "input", JsValue::str(input))?;
    if include_word_like { i.set_prop(&object, "isWordLike", JsValue::Bool(segment.word_like))?; }
    Ok(object)
}

fn intl_segments_containing(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("incompatible Segments receiver")); };
    let Some(IntlState::Segments(state)) = intl_state(object) else {
        return Err(Throw::type_error("incompatible Segments receiver"));
    };
    let number = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    // ToInteger: NaN and signed zero become +0, other values truncate toward zero.
    let index = if number.is_nan() { 0.0 } else { number.trunc() };
    if index < 0.0 { return Ok(JsValue::Undefined); }
    let index = index as usize;
    let Some(piece) = state.pieces.iter().find(|piece| piece.index <= index && index < piece.end) else {
        return Ok(JsValue::Undefined);
    };
    intl_segment_record(i, piece, &state.input, state.granularity == "word")
}

fn intl_segments_iterator(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("incompatible Segments receiver")); };
    let Some(IntlState::Segments(state)) = intl_state(object) else {
        return Err(Throw::type_error("incompatible Segments receiver"));
    };
    let mut raw = JsObject::ordinary();
    raw.proto = Some(i.object_proto.clone());
    let iterator = Obj(Rc::new(RefCell::new(raw)));
    intl_method(i, &iterator, "next", 0, intl_segment_iterator_next);
    iterator.borrow_mut().define(WellKnownSymbol::Iterator.symbol(),
        binary_writable_property(Interpreter::native("[Symbol.iterator]", intl_segment_iterator_self, &i.function_proto))).unwrap();
    intl_register(&iterator, IntlState::SegmentIterator(IntlSegmentIteratorState {
        input: state.input, granularity: state.granularity, pieces: state.pieces, cursor: Rc::new(RefCell::new(0)),
    }));
    Ok(JsValue::Object(iterator))
}

fn intl_segment_iterator_self(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("incompatible Segment iterator")); };
    if !matches!(intl_state(object), Some(IntlState::SegmentIterator(_))) {
        return Err(Throw::type_error("incompatible Segment iterator"));
    }
    Ok(this)
}

fn intl_segment_iterator_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("incompatible Segment iterator")); };
    let Some(IntlState::SegmentIterator(state)) = intl_state(object) else {
        return Err(Throw::type_error("incompatible Segment iterator"));
    };
    let index = *state.cursor.borrow();
    let result = i.object_value();
    if let Some(piece) = state.pieces.get(index) {
        *state.cursor.borrow_mut() = index + 1;
        let value = intl_segment_record(i, piece, &state.input, state.granularity == "word")?;
        i.set_prop(&result, "value", value)?;
        i.set_prop(&result, "done", JsValue::Bool(false))?;
    } else {
        i.set_prop(&result, "value", JsValue::Undefined)?;
        i.set_prop(&result, "done", JsValue::Bool(true))?;
    }
    Ok(result)
}

#[cfg(test)]
mod intl_segmenter_tests {
    use super::*;

    #[test]
    fn grapheme_and_word_segments_are_iterable() {
        let mut i = Interpreter::new();
        let value = i.run_script("let g=new Intl.Segmenter('en').segment('a\u{301}b'); let w=new Intl.Segmenter('en',{granularity:'word'}).segment('hi there'); [Array.from(g).map(x=>x.segment).join('|'),w.containing(3).segment].join(';')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "a\u{301}|b;there");
    }
}
