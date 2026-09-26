// Strict and loose equality shared by the tree walker and VM.

impl Interpreter {
    fn strict_eq(&self, a: &JsValue, b: &JsValue) -> bool {
        a == b
    }
    fn loose_eq(&mut self, a: &JsValue, b: &JsValue) -> Result<bool, Throw> {
        use JsValue::*;
        Ok(match (a, b) {
            (Undefined, Undefined) | (Null, Null) => true,
            (Bool(_), Bool(_)) | (Num(_), Num(_)) | (BigInt(_), BigInt(_))
            | (Str(_), Str(_)) | (Symbol(_), Symbol(_)) | (Object(_), Object(_)) => self.strict_eq(a, b),
            (Undefined, Null) | (Null, Undefined) => true,
            (Num(_), Str(_)) => self.loose_eq(a, &Num(b.to_number()?))?,
            (Str(_), Num(_)) => self.loose_eq(&Num(a.to_number()?), b)?,
            (BigInt(x), Str(y)) | (Str(y), BigInt(x)) =>
                BinaryBigInt::parse_string(&y.to_utf8_lossy()).is_some_and(|parsed| x.as_ref() == &parsed),
            (Bool(_), _) => {
                let n = a.to_number()?;
                self.loose_eq(&Num(n), b)?
            }
            (_, Bool(_)) => {
                let n = b.to_number()?;
                self.loose_eq(a, &Num(n))?
            }
            (Num(_) | BigInt(_) | Str(_) | Symbol(_), Object(_)) => {
                let p = self.primitive(b.clone())?;
                self.loose_eq(a, &p)?
            }
            (Object(_), Num(_) | BigInt(_) | Str(_) | Symbol(_)) => {
                let p = self.primitive(a.clone())?;
                self.loose_eq(&p, b)?
            }
            (BigInt(x), Num(y)) | (Num(y), BigInt(x)) =>
                Self::compare_bigint_number(x, *y) == Some(std::cmp::Ordering::Equal),
            (Null | Undefined, Object(object)) | (Object(object), Null | Undefined) =>
                object.borrow().is_html_dda,
            _ => false,
        })
    }
}
