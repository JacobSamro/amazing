//! HTML event-handler FunctionBody parsing, independent of the calling script.
use super::*;

/// Parse an ordinary handler body with host-supplied parameter names.
///
/// The body is parsed directly to EOF, so closing braces and comments cannot
/// escape a synthetic function wrapper. Strictness starts false and can be
/// enabled by the body's own directive prologue. `name` is metadata only.
pub fn parse_event_handler(
    name: &str,
    parameters: &[&str],
    source: &str,
) -> Result<FunctionItem, SyntaxError> {
    // Hashbang comments are a Script/Module prefix, not FunctionBody grammar.
    if source.starts_with("#!") {
        return Err(SyntaxError::new("hashbang is not allowed in a function body", 1, 1));
    }
    let mut parser = Parser::new(source);
    parser.in_function = 1;
    parser.in_new_target = 1;
    parser.block_depth = 1;
    let program = parser.parse_program()?;
    let body = Block { body: program.body, span: program.span };
    let mut params = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        if parameter.starts_with("#!") {
            return Err(SyntaxError::new("handler parameter must be one identifier", 1, 1));
        }
        let mut parser = Parser::new(parameter);
        let identifier = parser.expect_ident()?;
        if !parser.at_eof() {
            return Err(parser.err("handler parameter must be one identifier"));
        }
        let span = identifier.span;
        params.push(Param { binding: Binding::Ident(identifier), default: None, rest: false, span });
    }
    Parser::new(source).validate_function_params(&params, &body, false)?;
    Ok(FunctionItem {
        name: Some(Ident::new(name, body.span)), params, body,
        is_async: false, is_generator: false,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_event_handler;

    #[test]
    fn handler_body_has_an_independent_function_grammar_goal() {
        for source in ["return event; // eof", "return new.target;", "var yield = 1; return yield;", "return function() { return event; };"] {
            assert!(parse_event_handler("onclick", &["event"], source).is_ok(), "{source}");
        }
        for source in ["#!/usr/bin/env node\nreturn event;", "} globalThis.injected = true; {", "await event;", "yield 1;", "super.x;", "let event;", "'use strict'; with (event) {}", "'use strict'; delete event;"] {
            assert!(parse_event_handler("onclick", &["event"], source).is_err(), "{source}");
        }
        assert!(parse_event_handler("onclick", &["event) {}"], "").is_err());
        assert!(parse_event_handler("onclick", &["#!comment\nevent"], "").is_err());
        assert!(parse_event_handler("onerror", &["event", "source", "lineno", "colno", "error"], "return error;").is_ok());
    }
}
