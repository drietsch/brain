//! Compact term notation — a dense authoring/projection surface.
//!
//! Run 2 of the authoring experiment found that JSON emission validity holds
//! to ~90 nodes but the encoding is punishing (~250 lines of JSON for a
//! 10-line program). This notation is the response: an S-expression surface
//! that parses into the *same canonical Term* — identity, hashing, and dedup
//! are untouched because they live below the encoding. A program authored in
//! notation and the same program authored in JSON are one node in the graph.
//!
//! Grammar (whitespace-insensitive; `;` comments to end of line):
//!
//! ```text
//! 42  -7  "text"  true  false  unit      literals
//! name                                   variable
//! (lam x <body>)                         abstraction
//! (app <f> <x>)                          application
//! (let x <value> <body>)                 binding
//! {field <expr> field <expr>}            record
//! (get <record> field)                   field access
//! (tag name <payload>)                   variant construction
//! (match <scrut> (case tag bind <body>)... (else <body>)?)
//! (ref b3:<hash>)                        reference by content hash
//! (hole id expected?)                    typed hole
//! (<symbol/with/slash> <arg>)            foreign call
//! (add a b) (sub a b) (mul a b)          sugar for core/* foreign calls
//! (lt a b) (eq a b) (concat a b)         ... with record {a, b}
//! (if <cond> <then> <else>)              sugar for core/if {cond, then, else}
//! ```

use brain_core::ids::NodeId;
use brain_core::object::{Arm, Literal, Term};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    L,
    R,
    LB,
    RB,
    Str(String),
    Atom(String),
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            '(' => { chars.next(); out.push(Tok::L); }
            ')' => { chars.next(); out.push(Tok::R); }
            '{' => { chars.next(); out.push(Tok::LB); }
            '}' => { chars.next(); out.push(Tok::RB); }
            ';' => {
                for c2 in chars.by_ref() {
                    if c2 == '\n' { break; }
                }
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => return Err("unterminated string".to_string()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            other => return Err(format!("bad escape: {other:?}")),
                        },
                        Some(ch) => s.push(ch),
                    }
                }
                out.push(Tok::Str(s));
            }
            c if c.is_whitespace() => { chars.next(); }
            _ => {
                let mut a = String::new();
                while let Some(&c2) = chars.peek() {
                    if c2.is_whitespace() || "(){}\";".contains(c2) { break; }
                    a.push(c2);
                    chars.next();
                }
                out.push(Tok::Atom(a));
            }
        }
    }
    Ok(out)
}

pub fn parse_term(src: &str) -> Result<Term, String> {
    let mut p = Parser { toks: tokenize(src)?, pos: 0 };
    let term = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(format!("trailing input after term (token {})", p.pos));
    }
    Ok(term)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn next(&mut self) -> Result<Tok, String> {
        let t = self.toks.get(self.pos).cloned().ok_or("unexpected end of input")?;
        self.pos += 1;
        Ok(t)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn atom(&mut self) -> Result<String, String> {
        match self.next()? {
            Tok::Atom(a) => Ok(a),
            other => Err(format!("expected identifier, found {other:?}")),
        }
    }

    fn close(&mut self) -> Result<(), String> {
        match self.next()? {
            Tok::R => Ok(()),
            other => Err(format!("expected ')', found {other:?}")),
        }
    }

    fn expr(&mut self) -> Result<Term, String> {
        match self.next()? {
            Tok::Str(s) => Ok(Term::Lit { value: Literal::Str { value: s } }),
            Tok::Atom(a) => atom_term(&a),
            Tok::L => self.form(),
            Tok::LB => self.record(),
            other => Err(format!("unexpected {other:?}")),
        }
    }

    fn record(&mut self) -> Result<Term, String> {
        let mut fields = BTreeMap::new();
        loop {
            match self.peek() {
                Some(Tok::RB) => { self.pos += 1; break; }
                _ => {
                    let name = self.atom()?;
                    let value = self.expr()?;
                    fields.insert(name, value);
                }
            }
        }
        Ok(Term::Record { fields })
    }

    fn form(&mut self) -> Result<Term, String> {
        let head = self.atom()?;
        let term = match head.as_str() {
            "lam" => {
                let param = self.atom()?;
                let body = self.expr()?;
                Term::Lam { param, body: Box::new(body) }
            }
            "app" => {
                let func = self.expr()?;
                let arg = self.expr()?;
                Term::App { func: Box::new(func), arg: Box::new(arg) }
            }
            "let" => {
                let name = self.atom()?;
                let value = self.expr()?;
                let body = self.expr()?;
                Term::Let { name, value: Box::new(value), body: Box::new(body) }
            }
            "get" => {
                let record = self.expr()?;
                let field = self.atom()?;
                Term::Field { record: Box::new(record), field }
            }
            "tag" => {
                let tag = self.atom()?;
                let payload = self.expr()?;
                Term::Variant { tag, payload: Box::new(payload) }
            }
            "ref" => {
                let id = self.atom()?;
                Term::RefNode { node: NodeId::parse(&id).map_err(|e| e.to_string())? }
            }
            "hole" => {
                let id = self.atom()?;
                let expected = match self.peek() {
                    Some(Tok::Atom(_)) | Some(Tok::Str(_)) => Some(match self.next()? {
                        Tok::Atom(a) => a,
                        Tok::Str(s) => s,
                        _ => unreachable!(),
                    }),
                    _ => None,
                };
                Term::Hole { id, expected }
            }
            "foreign" => {
                let symbol = self.atom()?;
                let arg = self.expr()?;
                Term::Foreign { symbol, arg: Box::new(arg) }
            }
            "if" => {
                let mut fields = BTreeMap::new();
                fields.insert("cond".to_string(), self.expr()?);
                fields.insert("then".to_string(), self.expr()?);
                fields.insert("else".to_string(), self.expr()?);
                Term::Foreign {
                    symbol: "core/if".to_string(),
                    arg: Box::new(Term::Record { fields }),
                }
            }
            "add" | "sub" | "mul" | "lt" | "eq" | "concat" => {
                let mut fields = BTreeMap::new();
                fields.insert("a".to_string(), self.expr()?);
                fields.insert("b".to_string(), self.expr()?);
                Term::Foreign {
                    symbol: format!("core/{head}"),
                    arg: Box::new(Term::Record { fields }),
                }
            }
            "match" => {
                let scrutinee = self.expr()?;
                let mut arms = BTreeMap::new();
                let mut default = None;
                loop {
                    match self.peek() {
                        Some(Tok::R) => break,
                        Some(Tok::L) => {
                            self.pos += 1;
                            match self.atom()?.as_str() {
                                "case" => {
                                    let tag = self.atom()?;
                                    let bind = self.atom()?;
                                    let body = self.expr()?;
                                    self.close()?;
                                    arms.insert(tag, Arm { bind, body });
                                }
                                "else" => {
                                    let body = self.expr()?;
                                    self.close()?;
                                    default = Some(Box::new(body));
                                }
                                other => return Err(format!("expected case/else in match, found '{other}'")),
                            }
                        }
                        other => return Err(format!("expected case/else in match, found {other:?}")),
                    }
                }
                Term::Match { scrutinee: Box::new(scrutinee), arms, default }
            }
            h if h.contains('/') => {
                let arg = self.expr()?;
                Term::Foreign { symbol: h.to_string(), arg: Box::new(arg) }
            }
            other => return Err(format!("unknown form '{other}'")),
        };
        self.close()?;
        Ok(term)
    }
}

fn atom_term(a: &str) -> Result<Term, String> {
    if let Ok(i) = a.parse::<i64>() {
        return Ok(Term::Lit { value: Literal::Int { value: i } });
    }
    match a {
        "true" => Ok(Term::Lit { value: Literal::Bool { value: true } }),
        "false" => Ok(Term::Lit { value: Literal::Bool { value: false } }),
        "unit" => Ok(Term::Lit { value: Literal::Unit }),
        _ if a.contains('/') => Err(format!(
            "foreign symbol '{a}' must be applied: ({a} <arg>)"
        )),
        _ => Ok(Term::Var { name: a.to_string() }),
    }
}

// ---------------------------------------------------------------------------
// Printer: Term -> notation (the projection direction)
// ---------------------------------------------------------------------------

pub fn print_term(t: &Term) -> String {
    render(t, 0)
}

const WIDTH: usize = 80;

fn form(pieces: Vec<String>, indent: usize) -> String {
    let inline = format!("({})", pieces.join(" "));
    if indent + inline.len() <= WIDTH && !inline.contains('\n') {
        return inline;
    }
    let pad = " ".repeat(indent + 2);
    let mut s = format!("({}", pieces[0]);
    for p in &pieces[1..] {
        s.push('\n');
        s.push_str(&pad);
        s.push_str(p);
    }
    s.push(')');
    s
}

fn lit_str(l: &Literal) -> String {
    match l {
        Literal::Int { value } => value.to_string(),
        Literal::Bool { value } => value.to_string(),
        Literal::Unit => "unit".to_string(),
        Literal::Str { value } => {
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t");
            format!("\"{escaped}\"")
        }
    }
}

fn render(t: &Term, ind: usize) -> String {
    match t {
        Term::Lit { value } => lit_str(value),
        Term::Var { name } => name.clone(),
        Term::RefNode { node } => format!("(ref {node})"),
        Term::Hole { id, expected } => match expected {
            Some(e) => format!("(hole {id} {e})"),
            None => format!("(hole {id})"),
        },
        Term::Lam { param, body } => {
            form(vec!["lam".to_string(), param.clone(), render(body, ind + 2)], ind)
        }
        Term::App { func, arg } => form(
            vec!["app".to_string(), render(func, ind + 2), render(arg, ind + 2)],
            ind,
        ),
        Term::Let { name, value, body } => form(
            vec![
                "let".to_string(),
                name.clone(),
                render(value, ind + 2),
                render(body, ind + 2),
            ],
            ind,
        ),
        Term::Field { record, field } => form(
            vec!["get".to_string(), render(record, ind + 2), field.clone()],
            ind,
        ),
        Term::Variant { tag, payload } => form(
            vec!["tag".to_string(), tag.clone(), render(payload, ind + 2)],
            ind,
        ),
        Term::Record { fields } => {
            let pieces: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k} {}", render(v, ind + 2)))
                .collect();
            let inline = format!("{{{}}}", pieces.join(" "));
            if ind + inline.len() <= WIDTH && !inline.contains('\n') {
                inline
            } else {
                let pad = " ".repeat(ind + 2);
                format!("{{{}}}", pieces.join(&format!("\n{pad}")))
            }
        }
        Term::Match { scrutinee, arms, default } => {
            let mut pieces = vec!["match".to_string(), render(scrutinee, ind + 2)];
            for (tag, arm) in arms {
                pieces.push(form(
                    vec![
                        "case".to_string(),
                        tag.clone(),
                        arm.bind.clone(),
                        render(&arm.body, ind + 4),
                    ],
                    ind + 2,
                ));
            }
            if let Some(d) = default {
                pieces.push(form(vec!["else".to_string(), render(d, ind + 4)], ind + 2));
            }
            form(pieces, ind)
        }
        Term::Foreign { symbol, arg } => render_foreign(symbol, arg, ind),
    }
}

fn render_foreign(symbol: &str, arg: &Term, ind: usize) -> String {
    if let Term::Record { fields } = arg {
        if symbol == "core/if" && fields.len() == 3 {
            if let (Some(c), Some(t), Some(e)) =
                (fields.get("cond"), fields.get("then"), fields.get("else"))
            {
                return form(
                    vec![
                        "if".to_string(),
                        render(c, ind + 2),
                        render(t, ind + 2),
                        render(e, ind + 2),
                    ],
                    ind,
                );
            }
        }
        if let Some(short) = symbol.strip_prefix("core/") {
            if matches!(short, "add" | "sub" | "mul" | "lt" | "eq" | "concat")
                && fields.len() == 2
            {
                if let (Some(a), Some(b)) = (fields.get("a"), fields.get("b")) {
                    return form(
                        vec![short.to_string(), render(a, ind + 2), render(b, ind + 2)],
                        ind,
                    );
                }
            }
        }
    }
    form(vec![symbol.to_string(), render(arg, ind + 2)], ind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(t: &Term) {
        let printed = print_term(t);
        let parsed = parse_term(&printed).expect(&printed);
        assert_eq!(&parsed, t, "roundtrip failed for:\n{printed}");
    }

    #[test]
    fn parses_sugar_into_canonical_foreign_calls() {
        let t = parse_term("(add 1 2)").unwrap();
        match &t {
            Term::Foreign { symbol, arg } => {
                assert_eq!(symbol, "core/add");
                match arg.as_ref() {
                    Term::Record { fields } => {
                        assert_eq!(fields.len(), 2);
                        assert!(fields.contains_key("a") && fields.contains_key("b"));
                    }
                    other => panic!("expected record arg, got {other:?}"),
                }
            }
            other => panic!("expected foreign, got {other:?}"),
        }
        roundtrip(&t);
    }

    #[test]
    fn notation_and_json_produce_identical_terms() {
        // The abs program, both encodings — must be the same Term (and
        // therefore the same content hash in the graph).
        let notation = "(lam n (if (lt n 0) (mul n -1) n))";
        let json = r#"{"op":"lam","param":"n","body":{"op":"foreign","symbol":"core/if",
            "arg":{"op":"record","fields":{
            "cond":{"op":"foreign","symbol":"core/lt","arg":{"op":"record","fields":{
                "a":{"op":"var","name":"n"},"b":{"op":"lit","value":{"type":"int","value":0}}}}},
            "then":{"op":"foreign","symbol":"core/mul","arg":{"op":"record","fields":{
                "a":{"op":"var","name":"n"},"b":{"op":"lit","value":{"type":"int","value":-1}}}}},
            "else":{"op":"var","name":"n"}}}}}"#;
        let from_notation = parse_term(notation).unwrap();
        let from_json: Term = serde_json::from_str(json).unwrap();
        assert_eq!(from_notation, from_json);
    }

    #[test]
    fn match_variant_record_string_roundtrip() {
        let src = r#"
            (lam input
              (match (get input event)
                (case emergency _ (tag red unit))
                (case tick t
                  (let msg (concat "state: " "tick")
                    (app (lam x x) {label msg count 2})))
                (else (tag red unit))))
        "#;
        let t = parse_term(src).unwrap();
        roundtrip(&t);
    }

    #[test]
    fn holes_refs_and_errors() {
        roundtrip(&parse_term("(hole h0 int)").unwrap());
        assert!(parse_term("(bogus 1)").unwrap_err().contains("unknown form"));
        assert!(parse_term("core/add").unwrap_err().contains("must be applied"));
        assert!(parse_term("(add 1 2) extra").unwrap_err().contains("trailing"));
        assert!(parse_term("(add 1").unwrap_err().contains("end of input"));
    }
}
