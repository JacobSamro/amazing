use super::*;
use crate::interp::Interpreter;
use crate::value::FunctionBody;

fn assert_source(code: &FunctionCode, expected: &[u16], original: &crate::source_text::SourceMetadata) {
    assert_eq!(code.0.source.units(), Some(expected));
    assert!(code.0.source.range().unwrap().text().same_source(original.range().unwrap().text()));
    let i=Interpreter::new();
    let JsValue::Object(value)=code.instantiate(&i,&i.global_env) else {panic!("callable")};
    let value=value.borrow();let f=value.func.as_ref().unwrap();
    assert!(matches!(&f.body,FunctionBody::Bytecode(_)));
    assert_eq!(f.source.units(),Some(expected));
    assert!(f.source.range().unwrap().text().same_source(original.range().unwrap().text()));
}

#[test]
fn genuine_native_function_source_survives_ast_drop_without_utf16_reconstruction() {
    for prefix in ["function","function*","async function","async function*"] {
        let mut units=format!("{prefix} f(){{/*").encode_utf16().collect::<Vec<_>>();
        units.extend([0xD800,0xDC00,0xDFFF]);units.extend("*/\r\nreturn 1}".encode_utf16());
        let mut input="/*outside*/\n".encode_utf16().collect::<Vec<_>>();input.extend_from_slice(&units);input.extend(";/*tail*/".encode_utf16());
        let ast=crate::parse_script_utf16(&input).unwrap();
        let crate::ast::StmtKind::Function(f)=&ast.body[0].kind else {panic!("function")};
        let original=f.source.clone();let code=compile_function_item(f,ast.body[0].span,false).unwrap();
        assert!(!code.0.program.code.iter().any(|op|matches!(op,Instruction::TreeEval(..))));
        drop(ast);drop(input);let clone=code.clone();drop(code);assert_source(&clone,&units,&original);
    }
}

#[test]
fn native_arrow_and_method_adapters_keep_actual_production_ranges() {
    for text in ["x /*parameter*/ => x+1","async (x) => { return x+1 }"] {
        let ast=crate::parse_script(&format!("/*outside*/({text});")).unwrap();
        let crate::ast::StmtKind::Expression(expression)=&ast.body[0].kind else {panic!("expression")};
        let mut expression=expression;while let crate::ast::ExprKind::Paren(inner)=&expression.kind {expression=inner;}
        let crate::ast::ExprKind::Arrow(arrow)=&expression.kind else {panic!("arrow")};
        let code=compile_arrow_item(arrow,expression.span,false).unwrap();assert_source(&code,&text.encode_utf16().collect::<Vec<_>>(),&arrow.source);
    }
    let ast=crate::parse_script("class C { /*outside*/ async *[\"m\"](/*parameter*/x){ return x } }").unwrap();
    let crate::ast::StmtKind::Class(class)=&ast.body[0].kind else {panic!("class")};
    let crate::ast::ClassMember::Method(method)=&class.members[0] else {panic!("method")};
    let expected="async *[\"m\"](/*parameter*/x){ return x }".encode_utf16().collect::<Vec<_>>();
    assert_source(&compile_class_method(method,true).unwrap(),&expected,&method.source);
    let mut compiler=Compiler::new(false);
    assert_source(&compiler.compile_method(method,true).unwrap(),&expected,&method.source);
}

#[test]
fn native_class_constructor_uses_whole_class_and_body_bridges_have_no_fabricated_source() {
    for text in ["class C { m(){return 1} }","class C { constructor(){} m(){return 1} }"] {
        let ast=crate::parse_script(text).unwrap();let crate::ast::StmtKind::Class(class)=&ast.body[0].kind else {panic!("class")};
        let mut compiler=Compiler::new(false);let code=compiler.compile_class(class,ast.body[0].span).unwrap();
        assert_source(&code.constructor,&text.encode_utf16().collect::<Vec<_>>(),&class.source);
    }
    let ast=crate::parse_script("function f(){return 1}").unwrap();let crate::ast::StmtKind::Function(f)=&ast.body[0].kind else {panic!("function")};
    for code in [compile_async_body(&f.body,false).unwrap(),compile_generator_body(&f.body,false).unwrap(),compile_async_generator_body(&f.body,false).unwrap()] {
        assert!(code.0.source.units().is_none());
    }
}
