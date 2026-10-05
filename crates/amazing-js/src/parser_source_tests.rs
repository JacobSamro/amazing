//! Exact source carrier/independent-goal groups; no Function.toString reconstruction.
use super::*;
use crate::source_text::{SourceMetadata, SourceText};
fn units(text: &str) -> Vec<u16> { text.encode_utf16().collect() }
fn exact(source: &SourceMetadata, text: &str) { assert_eq!(source.units(), Some(units(text).as_slice())); }
fn declaration(text: &str) -> FunctionItem {
    let p = parse_script(text).unwrap();
    let StmtKind::Function(f) = &p.body[0].kind else { panic!("function") }; (**f).clone()
}
#[test]
fn source_ranges_survive_clones_and_ignore_semantic_equality_and_hash() {
    use std::hash::{Hash, Hasher};
    let f = declaration("function f(a) { /* retained */ return a; }");
    let fresh = declaration("function f(a) { /* retained */ return a; }");
    assert_eq!(f, fresh);
    let original = f.source.range().unwrap();
    let cloned = f.clone();
    assert!(original.text().same_source(cloned.source.range().unwrap().text()));
    assert!(!original.text().same_source(fresh.source.range().unwrap().text()));
    let absent = SourceMetadata::default();
    assert_eq!(absent, f.source);
    fn hash(v: &SourceMetadata) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new(); v.hash(&mut h); h.finish()
    }
    assert_eq!(hash(&absent), hash(&f.source));
    let text = SourceText::from_utf16(&[0xd800, 0x61, 0xdc00]);
    assert!(text.range(2,1).is_none()); assert!(text.range(0,4).is_none());
    assert_eq!(text.range(0,3).unwrap().units(), &[0xd800,0x61,0xdc00]);
    let source = f.source.clone(); drop(f); drop(cloned);
    exact(&source, "function f(a) { /* retained */ return a; }");
    // Only the new immutable source carrier crosses this thread boundary.
    // This makes no claim about interpreter/AST Rc transfer soundness.
    let worker_source = source.clone();
    let worker = std::thread::spawn(move || {
        let copied = worker_source.clone();
        copied.units().unwrap().to_vec()
    });
    assert_eq!(worker.join().unwrap(), source.units().unwrap());
}
#[test]
fn all_function_and_arrow_productions_keep_actual_unit_boundaries() {
    for text in ["function f /* a */(a) { return /a/; }", "function* f(a){ yield a; }",
        "async /*line*/ function f(a){ await a; }", "async function* f(a){ yield await a; }"] {
        let f = declaration(&format!("/*lead*/{text} //tail")); exact(&f.source,text);
    }
    for text in ["x => ({x})", "(x=1) => { return x; }", "async x => await x", "async (x) => x"] {
        let p=parse_script(&format!("({text});")).unwrap();
        let StmtKind::Expression(expr)=&p.body[0].kind else {panic!("expr")};
        let ExprKind::Paren(inner)=&expr.kind else {panic!("paren")};
        let ExprKind::Arrow(arrow)=&inner.kind else {panic!("arrow")}; exact(&arrow.source,text);
    }
    let p=parse_script("(async function /*x*/ f(){return 1;});").unwrap();
    let StmtKind::Expression(e)=&p.body[0].kind else {panic!("expr")};
    let ExprKind::Paren(e)=&e.kind else {panic!("paren")};
    let ExprKind::Function(f)=&e.kind else {panic!("function")}; exact(&f.source,"async function /*x*/ f(){return 1;}");
}
#[test]
fn classes_methods_and_regex_template_rescans_keep_genuine_source() {
    let text="class C extends Object { constructor(){} static async *m(){yield 1;} get ['x'](){return 2;} #p(){} }";
    let p=parse_script(text).unwrap(); let StmtKind::Class(c)=&p.body[0].kind else {panic!("class")};
    exact(&c.source,text);
    let expected=["constructor(){}","async *m(){yield 1;}","get ['x'](){return 2;}","#p(){}"];
    for (m,text) in c.members.iter().zip(expected) { let ClassMember::Method(m)=m else {panic!("method")}; exact(&m.source,text); }
    let text="function f(){ return /[}]/u.test(`a${(()=>`b${1}`)()}c`); }";
    exact(&declaration(text).source,text);
    let p=parse_script("({async *m(){yield 1;}, get [name](){return 2;}, set x(v){}});").unwrap();
    let StmtKind::Expression(e)=&p.body[0].kind else {panic!("expr")};
    let ExprKind::Paren(e)=&e.kind else {panic!("paren")}; let ExprKind::Object(elements)=&e.kind else {panic!("object")};
    for (e,text) in elements.iter().zip(["async *m(){yield 1;}","get [name](){return 2;}","set x(v){}"]) {
        let ObjElem::Prop(p)=e else {panic!("prop")}; let PropValue::Method(m)=&p.value else {panic!("method")}; exact(&m.source,text);
    }
}
#[test]
fn source_units_preserve_crlf_surrogates_and_template_site_identity() {
    let mut text=units("function f(){\r\nreturn '"); text.extend([0xd800,0xdc00,0xdfff]); text.extend(units("';\r\n}"));
    let p=parse_script_utf16(&text).unwrap(); let StmtKind::Function(f)=&p.body[0].kind else {panic!("function")}; assert_eq!(f.source.units(),Some(text.as_slice()));
    let mut lexer=crate::lexer::Lexer::new_utf16(&units("/*😀*/`a${1}b`"));
    let part=lexer.next_token().unwrap(); assert_eq!(part.source.units(),Some(units("`a${").as_slice()));
    let delimiter=lexer.next_token().unwrap(); assert_eq!(delimiter.source.units(),Some(units("${").as_slice()));
    let token=lexer.next_token().unwrap(); assert_eq!(token.source.units(),Some(units("1").as_slice()));
    let token=lexer.next_token().unwrap(); assert_eq!(token.source.units(),Some(units("}b`").as_slice()));
    let eof=lexer.next_token().unwrap(); assert_eq!(eof.source.units(),Some(&[][..]));
    let p=parse_script("function f(){return tag`x`;}").unwrap(); let clone=p.clone(); assert_eq!(p,clone);
}
#[test]
fn independent_parameter_and_body_goals_reject_cross_boundary_fragments() {
    for kind in [DynamicFunctionKind::Normal,DynamicFunctionKind::Generator,DynamicFunctionKind::Async,DynamicFunctionKind::AsyncGenerator] {
        for text in ["", "a,b=2,...rest", "{x},[a,,b]", "a = 1 in {}"] {assert!(parse_dynamic_parameters_utf16(&units(text),kind).is_ok(),"{kind:?}: {text}");}
        for text in ["/*", "a) {}", "...rest,", "a={x=1}", "#!/x\na", "a=1,a", "{x},x", "[a],a", "a,...a"] {assert!(parse_dynamic_parameters_utf16(&units(text),kind).is_err(),"{kind:?}: {text}");}
        for text in ["return 1;", "return new.target;", "'use strict'; return 2;", "var a; function f(){}"] {assert!(parse_dynamic_body_utf16(&units(text),kind).is_ok(),"{kind:?}: {text}");}
        for text in ["*/ ) {", "} injected(); {", "#!/x\nreturn 1;", "super.x;", "this.#x;", "break;", "'use strict'; with({}){}"] {assert!(parse_dynamic_body_utf16(&units(text),kind).is_err(),"{kind:?}: {text}");}
    }
    assert!(parse_dynamic_parameters_utf16(&units("yield"),DynamicFunctionKind::Generator).is_err());
    assert!(parse_dynamic_parameters_utf16(&units("await"),DynamicFunctionKind::Async).is_err());
    assert!(parse_dynamic_body_utf16(&units("yield 1;"),DynamicFunctionKind::Normal).is_err());
    assert!(parse_dynamic_body_utf16(&units("yield 1;"),DynamicFunctionKind::Generator).is_ok());
    assert!(parse_dynamic_body_utf16(&units("await 1;"),DynamicFunctionKind::Async).is_ok());
}
#[test]
fn complete_dynamic_expression_goal_checks_combined_early_errors_and_exact_source() {
    for (kind,prefix) in [(DynamicFunctionKind::Normal,"function"),(DynamicFunctionKind::Generator,"function*"),(DynamicFunctionKind::Async,"async function"),(DynamicFunctionKind::AsyncGenerator,"async function*")] {
        let text=format!("{prefix} anonymous(a\n) {{\nreturn a;\n}}"); let f=parse_dynamic_function_expression_utf16(&units(&text),kind).unwrap(); exact(&f.source,&text);
        for (params,body) in [("a=1","'use strict';"),("a","let a;"),("a={x=1}","return a;")] {
            let text=format!("{prefix} anonymous({params}\n) {{\n{body}\n}}"); assert!(parse_dynamic_function_expression_utf16(&units(&text),kind).is_err(),"{text}");
        }
        assert!(parse_dynamic_function_expression_utf16(&units(&format!("{text};")),kind).is_err());
    }
    for (kind,prefix) in [(DynamicFunctionKind::Normal,"function"),(DynamicFunctionKind::Generator,"function*"),(DynamicFunctionKind::Async,"async function"),(DynamicFunctionKind::AsyncGenerator,"async function*")] {
        assert!(parse_dynamic_parameters_utf16(&units("a,a"),kind).is_ok());
        let text=format!("{prefix} anonymous(a,a\n){{return a;}}");
        assert!(parse_dynamic_function_expression_utf16(&units(&text),kind).is_ok(),"{text}");
        for (params,body) in [("a,a","'use strict'; return a;"),("a=1,a","return a;"),("{a},a","return a;")] {
            let text=format!("{prefix} anonymous({params}\n){{{body}}}");
            assert!(parse_dynamic_function_expression_utf16(&units(&text),kind).is_err(),"{text}");
        }
    }
}

#[test]
fn every_real_token_creator_uses_cursor_units_and_synthetic_tokens_stay_unknown() {
    use crate::tokens::{Token,TokenKind,Punct,SourceLocation,Span};
    let source="#!script\n/*😀*/function \\u0066 #\\u0070 12.5e+2 0x12n 'x\\uD800' ?. =>";
    let mut lexer=crate::lexer::Lexer::new_utf16(&units(source));
    for text in ["function","\\u0066","#\\u0070","12.5e+2","0x12n","'x\\uD800'","?.","=>",""] {
        let token=lexer.next_token().unwrap(); exact(&token.source,text);
    }
    let at=SourceLocation::new(1,1);
    let unknown=Token::new(TokenKind::Invalid(SyntaxError::new("synthetic",1,1)),Span::new(at,at),false);
    assert!(unknown.source.range().is_none());
    assert!(Token::new(TokenKind::Punct(Punct::LParen),Span::new(at,at),false).source.range().is_none());
    let mut stream=crate::token_stream::TokenStream::new(" /*😀*/ /a/ + 1");
    let slash=stream.get(0);exact(&slash.source,"/");
    let original=slash.source.range().unwrap().text().clone();
    let _=stream.get(4); // scanner has speculated beyond the division goal
    stream.regex(0).unwrap();
    let regex=stream.get(0);exact(&regex.source,"/a/");
    assert!(original.same_source(regex.source.range().unwrap().text()));
    exact(&stream.get(1).source,"+");exact(&stream.get(2).source,"1");
}

#[test]
fn speculative_arrows_module_defaults_and_contextual_method_keys_keep_exact_syntax() {
    for text in ["(a=1) => ((a))","async (a = /a/.test('a')) => (`x${a}`)"] {
        let p=parse_script(&format!("({text});")).unwrap();
        let StmtKind::Expression(e)=&p.body[0].kind else {panic!("expr")};
        let ExprKind::Paren(e)=&e.kind else {panic!("paren")};let ExprKind::Arrow(a)=&e.kind else {panic!("arrow")};exact(&a.source,text);
    }
    // Failed arrow-cover speculation rewinds to the same actual token sequence.
    let p=parse_script("(function /*kept*/ f(){return /a/;});").unwrap();
    let StmtKind::Expression(e)=&p.body[0].kind else {panic!("expr")};let ExprKind::Paren(e)=&e.kind else {panic!("paren")};
    let ExprKind::Function(f)=&e.kind else {panic!("function")};exact(&f.source,"function /*kept*/ f(){return /a/;}");
    for text in ["function(){}","function*(){yield 1;}","async function(){return 1;}","async function*(){yield 1;}"] {
        let module=parse_module(&format!("export default {text}")).unwrap();
        let ModuleItem::ExportDefaultDeclaration(stmt)=&module.items[0] else {panic!("default")};
        let StmtKind::Function(f)=&stmt.kind else {panic!("function")};exact(&f.source,text);
    }
    for text in ["class {}","class C { static(){} get(){} set(){} async(){} ['\\uD800'](){} }"] {
        let module=parse_module(&format!("export default {text}")).unwrap();
        let ModuleItem::ExportDefaultDeclaration(stmt)=&module.items[0] else {panic!("default")};
        let StmtKind::Class(c)=&stmt.kind else {panic!("class")};exact(&c.source,text);
    }
    let p=parse_script("class C { static /*modifier*/ get x(){return 1;} static(){} }").unwrap();
    let StmtKind::Class(c)=&p.body[0].kind else {panic!("class")};
    for (member,text) in c.members.iter().zip(["get x(){return 1;}","static(){}"]) {
        let ClassMember::Method(m)=member else {panic!("method")};exact(&m.source,text);
    }
    let p=parse_script("@identity class C { @identity m(){} }").unwrap();
    let StmtKind::Class(c)=&p.body[0].kind else {panic!("class")};exact(&c.source,"@identity class C { @identity m(){} }");
    let ClassMember::Method(m)=&c.members[0] else {panic!("method")};exact(&m.source,"m(){}");
}
