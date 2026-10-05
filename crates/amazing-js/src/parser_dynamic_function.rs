//! Independent ParseText goals for CreateDynamicFunction. Source assembly is the host's job.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicFunctionKind { Normal, Generator, Async, AsyncGenerator }
impl DynamicFunctionKind {
    fn flags(self) -> (bool, bool) {
        (matches!(self, Self::Async | Self::AsyncGenerator),
         matches!(self, Self::Generator | Self::AsyncGenerator))
    }
    fn parser(self, units: &[u16]) -> Parser {
        let mut parser = Parser::new_utf16(units);
        parser.tokens.borrow_mut().set_function_goal();
        let (is_async, is_generator) = self.flags();
        parser.in_async = usize::from(is_async);
        parser.in_generator = usize::from(is_generator);
        parser.in_new_target = 1;
        parser
    }
}

pub fn parse_dynamic_parameters_utf16(units: &[u16], kind: DynamicFunctionKind) -> Result<Vec<Param>, SyntaxError> {
    let mut parser = kind.parser(units);
    parser.in_formal_params = 1;
    let params = parser.parse_parameter_list(true)?;
    if !parser.at_eof() { return Err(parser.err("expected end of FormalParameters")); }
    cover::parameters(&params)?;
    parser.validate_parameter_names(&params, false, false)?;
    Ok(params)
}

pub fn parse_dynamic_body_utf16(units: &[u16], kind: DynamicFunctionKind) -> Result<Block, SyntaxError> {
    let mut parser = kind.parser(units);
    parser.in_function = 1;
    parser.block_depth = 1;
    parser.parse_eof_statement_list()
}

pub fn parse_dynamic_function_expression_utf16(units: &[u16], kind: DynamicFunctionKind) -> Result<FunctionItem, SyntaxError> {
    // This goal has no enclosing function context; the expression establishes its own.
    let mut parser = Parser::new_utf16(units);
    parser.tokens.borrow_mut().set_function_goal();
    let (is_async, is_generator) = kind.flags();
    if is_async {
        if !parser.word("async") || parser.nth(1).newline_before {
            return Err(parser.err("expected AsyncFunctionExpression"));
        }
        parser.bump();
    }
    if !parser.at_kw(crate::tokens::Keyword::Function) {
        return Err(parser.err("expected FunctionExpression"));
    }
    let function = parser.parse_function_rest(is_async, false)?;
    if function.is_generator != is_generator || !parser.at_eof() {
        return Err(parser.err("function expression does not match the requested grammar goal"));
    }
    cover::function_item(&function)?;
    Ok(function)
}
