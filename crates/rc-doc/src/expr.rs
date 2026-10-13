//! Tiny arithmetic expression evaluator for numeric input fields.
//!
//! Supports `+ - * / ^`, parentheses, unary minus, `pi`, a trailing `mm`, and `,` as decimal
//! separator (so both `2.5` and `2,5` work).

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid expression: {0}")]
pub struct ExprError(String);

pub fn eval(input: &str) -> Result<f64, ExprError> {
    let s = input.trim();
    let s = s.strip_suffix("mm").unwrap_or(s).replace(',', ".");
    let mut p = Parser {
        chars: s.chars().filter(|c| !c.is_whitespace()).collect(),
        pos: 0,
    };
    let v = p.expr()?;
    if p.pos != p.chars.len() {
        return Err(ExprError(format!("unexpected '{}'", p.chars[p.pos])));
    }
    if v.is_finite() {
        Ok(v)
    } else {
        Err(ExprError("result is not a finite number".into()))
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn expr(&mut self) -> Result<f64, ExprError> {
        let mut v = self.term()?;
        while let Some(c @ ('+' | '-')) = self.peek() {
            self.pos += 1;
            let r = self.term()?;
            v = if c == '+' { v + r } else { v - r };
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, ExprError> {
        let mut v = self.power()?;
        while let Some(c @ ('*' | '/')) = self.peek() {
            self.pos += 1;
            let r = self.power()?;
            v = if c == '*' { v * r } else { v / r };
        }
        Ok(v)
    }

    fn power(&mut self) -> Result<f64, ExprError> {
        let base = self.unary()?;
        if self.peek() == Some('^') {
            self.pos += 1;
            let e = self.power()?;
            return Ok(base.powf(e));
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<f64, ExprError> {
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                Ok(-self.unary()?)
            }
            Some('+') => {
                self.pos += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<f64, ExprError> {
        match self.peek() {
            Some('(') => {
                self.pos += 1;
                let v = self.expr()?;
                if self.peek() != Some(')') {
                    return Err(ExprError("missing ')'".into()));
                }
                self.pos += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let start = self.pos;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E')
                {
                    // Allow exponent sign: 1e-3
                    if matches!(self.peek(), Some('e' | 'E'))
                        && matches!(self.chars.get(self.pos + 1), Some('-' | '+'))
                    {
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                let text: String = self.chars[start..self.pos].iter().collect();
                text.parse()
                    .map_err(|_| ExprError(format!("bad number '{text}'")))
            }
            Some('p') if self.chars.get(self.pos + 1) == Some(&'i') => {
                self.pos += 2;
                Ok(std::f64::consts::PI)
            }
            Some(c) => Err(ExprError(format!("unexpected '{c}'"))),
            None => Err(ExprError("unexpected end".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::eval;

    #[test]
    fn arithmetic() {
        assert_eq!(eval("20/2+3"), Ok(13.0));
        assert_eq!(eval(" -(1 + 2) * 3 "), Ok(-9.0));
        assert_eq!(eval("2^3^2"), Ok(512.0));
        assert_eq!(eval("2,5"), Ok(2.5));
        assert_eq!(eval("12mm"), Ok(12.0));
        assert_eq!(eval("1e-3*1000"), Ok(1.0));
        assert!((eval("2*pi").unwrap() - std::f64::consts::TAU).abs() < 1e-12);
    }

    #[test]
    fn errors() {
        assert!(eval("").is_err());
        assert!(eval("1+").is_err());
        assert!(eval("(1").is_err());
        assert!(eval("1/0").is_err());
        assert!(eval("abc").is_err());
    }
}
