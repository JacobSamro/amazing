//! Grid grammar consumes component values, not whitespace-delimited words.
//! A line-name block or function ends at its matching delimiter even when the
//! following track has no intervening whitespace. Borrow the original tokens
//! so strings, escaped identifiers and nested function contents stay intact.
use crate::syntax::Token;

pub(super) fn grid_components(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut components = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if matches!(tokens[index], Token::Whitespace) {
            index += 1;
            continue;
        }
        let start = index;
        let names = matches!(tokens[index], Token::OpenSquare);
        if matches!(tokens[index], Token::Function(_)) {
            index += 1;
            if !matches!(tokens.get(index), Some(Token::OpenParen)) {
                return None;
            }
        }
        let mut stack = Vec::new();
        match tokens[index] {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => return None,
            _ => {}
        }
        index += 1;
        while !stack.is_empty() {
            match tokens.get(index)? {
                Token::OpenParen => stack.push(')'),
                Token::OpenSquare => stack.push(']'),
                Token::OpenCurly => stack.push('}'),
                Token::CloseParen if stack.pop() != Some(')') => return None,
                Token::CloseSquare if stack.pop() != Some(']') => return None,
                Token::CloseCurly if stack.pop() != Some('}') => return None,
                // The tokenizer represents a function name and its opening
                // parenthesis separately; both remain inside this block.
                Token::Function(_) if !matches!(tokens.get(index + 1), Some(Token::OpenParen)) => {
                    return None;
                }
                _ => {}
            }
            index += 1;
            // Preserve the existing contiguous `[a][b]` line-name group.
            // Track/function boundaries still end the component immediately.
            if stack.is_empty() && names && matches!(tokens.get(index), Some(Token::OpenSquare)) {
                stack.push(']');
                index += 1;
            }
        }
        components.push(&tokens[start..index]);
    }
    Some(components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{serialize, tokenize};

    #[test]
    fn blocks_functions_and_primitives_are_atomic_without_spaces() {
        let tokens = tokenize("[one]50px[two]minmax(10px,calc(20px + 1%))[three]fit-content(50px)");
        let parts = grid_components(&tokens).unwrap();
        assert_eq!(parts.len(), 6);
        assert_eq!(serialize(parts[0]), "[one]");
        assert_eq!(serialize(parts[1]), "50px");
        assert_eq!(serialize(parts[4]), "[three]");
        assert!(serialize(parts[3]).starts_with("minmax("));
    }

    #[test]
    fn strings_and_nested_delimiters_keep_their_token_identity() {
        let tokens = tokenize("[start]\"a b\"50px[end]\"c d\"minmax(0,1fr)");
        let parts = grid_components(&tokens).unwrap();
        assert_eq!(parts.len(), 6);
        assert!(matches!(parts[1], [Token::Str(s)] if s == "a b"));
        for css in ["[a", "f([a)]", "[a}", ")", "f((1)"] {
            assert!(grid_components(&tokenize(css)).is_none(), "{css}");
        }
    }
}
